import { useCallback, useEffect, useState } from 'react';
import { motion, AnimatePresence } from 'motion/react';
import { getVersion } from '@tauri-apps/api/app';
import { open as openUrl } from '@tauri-apps/plugin-shell';
import {
  AlertTriangle, Bug, Cpu, HardDrive, Wifi, Monitor, Volume2,
  Usb, RefreshCw, ChevronDown, ChevronRight, ExternalLink,
  Copy, Check, Search, Zap, Shield, Terminal,
} from 'lucide-react';
import { Modal } from '../components/ui/Modal';
import { Button } from '../components/ui/Button';
import { Badge } from '../components/ui/Badge';
import { api } from '../bridge/invoke';
import { useHardware } from '../stores/hardware';
import { useEfi } from '../stores/efi';
import { useCompatibility } from '../stores/compatibility';
import { parseError } from '../lib/parseError';

interface TroubleshootProps {
  open: boolean;
  onClose: () => void;
}

// ── Knowledge base ─────────────────────────────────────────────────────────

interface TroubleEntry {
  id: string;
  title: string;
  icon: React.ReactNode;
  category: 'boot' | 'hardware' | 'post-install' | 'usb';
  symptoms: string[];
  causes: string[];
  fixes: string[];
  advanced?: string[];
  relatedKexts?: string[];
}

const TROUBLE_DB: TroubleEntry[] = [
  {
    id: 'black-screen',
    title: 'Black Screen on Boot',
    icon: <Monitor size={14} />,
    category: 'boot',
    symptoms: [
      'Screen goes black after Apple logo',
      'No signal on display after verbose boot',
      'Stuck at [EB|#LOG:EXITBS:START]',
    ],
    causes: [
      'Wrong ig-platform-id for your iGPU',
      'Discrete GPU not supported or needs agdpmod=pikera',
      'Broken HDMI/DP framebuffer patching',
    ],
    fixes: [
      'Add -v (verbose) boot arg to see where it stops',
      'Try agdpmod=pikera in boot-args for AMD dGPU + iMac SMBIOS',
      'For Intel iGPU: verify ig-platform-id matches your CPU generation',
      'For laptop dGPU: add -wegnoegpu to disable it',
      'Try a different display port (HDMI vs DisplayPort)',
    ],
    advanced: [
      'Check DeviceProperties > PciRoot for framebuffer patches',
      'Compare your ig-platform-id against Dortania guide values',
      'Enable WhateverGreen debug: -igfxdump -igfxfbdump',
    ],
    relatedKexts: ['WhateverGreen.kext', 'Lilu.kext'],
  },
  {
    id: 'kernel-panic',
    title: 'Kernel Panic',
    icon: <Zap size={14} />,
    category: 'boot',
    symptoms: [
      'White text on black screen with panic info',
      'System restarts during boot',
      'Verbose shows "panic" or "MACH exception"',
    ],
    causes: [
      'Missing or wrong kexts for your hardware',
      'AMD CPU patches not applied (core_count missing)',
      'SIP/SecureBootModel misconfigured',
      'ACPI errors from wrong SSDTs',
    ],
    fixes: [
      'Boot with keepsyms=1 debug=0x100 to see full panic log',
      'Verify all kexts match your macOS version',
      'AMD users: ensure core_count is set correctly in config',
      'Try SecureBootModel: Disabled temporarily',
      'Remove non-essential kexts and add back one by one',
    ],
    advanced: [
      'Analyze panic log: look for the faulting kext name',
      'Check Kernel > Quirks > ProvideCurrentCpuInfo for 12th+ gen Intel',
      'Verify SMBIOS matches CPU generation',
    ],
  },
  {
    id: 'stuck-verbose',
    title: 'Stuck During Verbose Boot',
    icon: <Terminal size={14} />,
    category: 'boot',
    symptoms: [
      'Boot hangs at a specific line in verbose mode',
      'Stuck at "IOConsoleUsers: gIOScreenLockState"',
      'Stuck at "End RandomSeed" or "PCI Configuration"',
    ],
    causes: [
      'Missing EC (Embedded Controller) SSDT',
      'USB port limit issue',
      'Missing XHCI handoff in BIOS',
    ],
    fixes: [
      'Identify the exact line where it stops',
      '"End RandomSeed": missing EC SSDT or USB issues',
      '"IOConsoleUsers": GPU configuration problem',
      '"PCI Configuration Begin": missing SSDT-PLUG or wrong CPU config',
      'Ensure XHCI Handoff is enabled in BIOS',
    ],
  },
  {
    id: 'no-wifi',
    title: 'Wi-Fi Not Working',
    icon: <Wifi size={14} />,
    category: 'hardware',
    symptoms: [
      'No Wi-Fi option in System Preferences',
      'Wi-Fi adapter not detected',
      'Can see networks but cannot connect',
    ],
    causes: [
      'Intel Wi-Fi needs itlwm.kext (not native)',
      'Broadcom card needs AirportBrcmFixup',
      'Wrong kext for macOS version',
    ],
    fixes: [
      'Intel cards: ensure itlwm.kext is loaded (check kext list)',
      'Download HeliPort app for Intel Wi-Fi management',
      'Broadcom: verify AirportBrcmFixup + correct patches',
      'If card is unsupported: consider a BCM94360CD swap ($15-25)',
      'Check System Report > Network for adapter visibility',
    ],
    relatedKexts: ['itlwm.kext', 'AirportItlwm.kext', 'AirportBrcmFixup.kext'],
  },
  {
    id: 'no-audio',
    title: 'No Audio Output',
    icon: <Volume2 size={14} />,
    category: 'hardware',
    symptoms: [
      'No audio devices in Sound preferences',
      'Audio device visible but no sound',
      'Only HDMI audio works',
    ],
    causes: [
      'Wrong layout-id for your codec',
      'AppleALC.kext not loaded',
      'Missing codec support',
    ],
    fixes: [
      'Check your codec model (ALC###) in hardware scan results',
      'Try different alcid= values (common: 1, 3, 5, 7, 11)',
      'Verify AppleALC.kext is in your kext list',
      'Reset audio: sudo killall coreaudiod',
      'Check System Report > Audio for device detection',
    ],
    relatedKexts: ['AppleALC.kext', 'Lilu.kext'],
  },
  {
    id: 'no-bluetooth',
    title: 'Bluetooth Not Working',
    icon: <Search size={14} />,
    category: 'hardware',
    symptoms: [
      'No Bluetooth icon in menu bar',
      'Bluetooth adapter not found',
      'Can pair but cannot connect',
    ],
    causes: [
      'Missing Bluetooth firmware kexts',
      'Intel BT needs IntelBluetoothFirmware',
      'Broadcom BT needs BrcmPatchRAM3',
    ],
    fixes: [
      'Intel: add IntelBluetoothFirmware.kext + BlueToolFixup.kext',
      'Broadcom: add BrcmPatchRAM3 + BrcmFirmwareData + BlueToolFixup',
      'macOS 12+: BlueToolFixup is required for all BT kexts',
      'Try resetting Bluetooth module: sudo pkill bluetoothd',
      'USB Bluetooth dongles usually work without kexts',
    ],
    relatedKexts: ['IntelBluetoothFirmware.kext', 'BlueToolFixup.kext', 'BrcmPatchRAM3.kext'],
  },
  {
    id: 'usb-issues',
    title: 'USB Ports Not Working',
    icon: <Usb size={14} />,
    category: 'usb',
    symptoms: [
      'Some USB ports not recognized',
      'USB 3.0 devices only run at 2.0 speed',
      'USB devices disconnect randomly',
      'Keyboard/mouse not working at boot',
    ],
    causes: [
      'macOS 15-port USB limit',
      'Missing USBInjectAll or XHCI-unsupported kext',
      'No USB port mapping done',
    ],
    fixes: [
      'Initial boot: USBInjectAll.kext handles discovery',
      'Add XhciPortLimit=true quirk if > 15 ports',
      'After install: create a proper USB map with USBMap tool',
      'For Haswell/Broadwell: add XHCI-unsupported.kext',
      'Verify XHCI Handoff is enabled in BIOS',
    ],
    advanced: [
      'Post-install USB mapping is CRITICAL for stability',
      'Use USBMap (Python) or USBToolBox (Windows) to create a custom map',
      'Map identifies which physical ports to keep (15 port limit)',
      'After mapping, replace USBInjectAll with your custom USBMap.kext',
      'Test all ports including USB-C, internal headers, and webcam',
    ],
    relatedKexts: ['USBInjectAll.kext', 'XHCI-unsupported.kext', 'USBMap.kext'],
  },
  {
    id: 'iservices',
    title: 'iMessage / FaceTime / App Store',
    icon: <Shield size={14} />,
    category: 'post-install',
    symptoms: [
      'Cannot sign in to iMessage',
      'FaceTime says "Could not sign in"',
      'App Store downloads fail',
    ],
    causes: [
      'Invalid or duplicate SMBIOS serial numbers',
      'ROM value not unique',
      'Network adapter not en0',
      'Apple has flagged the serial',
    ],
    fixes: [
      'Generate fresh SMBIOS serials (this app does it automatically)',
      'Verify serial is NOT valid on Apple check coverage page',
      'Ensure your Ethernet adapter is en0 (delete all in Network prefs, re-add)',
      'Set ROM to your real Ethernet MAC address',
      'Sign out of all Apple services, reset NVRAM, sign back in',
    ],
    advanced: [
      'If "Customer Code" error: call Apple support with a real Mac serial',
      'Check Apple ID account restrictions',
      'Try creating a new Apple ID on a real Mac/iOS device first',
    ],
  },
  {
    id: 'nvme-issues',
    title: 'NVMe / Storage Issues',
    icon: <HardDrive size={14} />,
    category: 'hardware',
    symptoms: [
      'NVMe drive not visible in installer',
      'Kernel panic mentioning IONVMeFamily',
      'Slow disk performance',
    ],
    causes: [
      'Known problematic NVMe controllers (Samsung PM981/PM991)',
      'Missing NVMeFix.kext',
      'TRIM not enabled',
    ],
    fixes: [
      'Add NVMeFix.kext to your EFI',
      'Samsung 970/980/990: NVMeFix usually resolves panics',
      'Intel 600p: may need to be used as secondary, not boot drive',
      'Enable SetApfsTrimTimeout quirk if APFS is slow',
      'Optane drives are NOT compatible \u2014 disable or remove',
    ],
    relatedKexts: ['NVMeFix.kext'],
  },
  {
    id: 'cpu-power',
    title: 'CPU Power Management',
    icon: <Cpu size={14} />,
    category: 'post-install',
    symptoms: [
      'CPU runs at full speed constantly',
      'Laptop battery drains very fast',
      'CPU stuck at minimum frequency',
      'High idle temperatures',
    ],
    causes: [
      'Missing CPUFriend.kext',
      'Wrong SMBIOS for your CPU',
      'SSDT-PLUG not loaded (pre-Monterey 12.3)',
    ],
    fixes: [
      'Add CPUFriend.kext + CPUFriendDataProvider.kext',
      'Generate CPUFriendDataProvider for your SMBIOS using CPUFriendFriend',
      'Verify SMBIOS matches your CPU generation',
      'Check CPU frequency with Intel Power Gadget or iStatMenus',
      'AMD: AMDRyzenCPUPowerManagement.kext handles this',
    ],
    relatedKexts: ['CPUFriend.kext', 'CPUFriendDataProvider.kext', 'AMDRyzenCPUPowerManagement.kext'],
  },
];

const CATEGORY_LABELS: Record<string, string> = {
  boot: 'Boot Issues',
  hardware: 'Hardware',
  'post-install': 'Post-Install',
  usb: 'USB',
};

const CATEGORY_COLORS: Record<string, string> = {
  boot: '#ef4444',
  hardware: '#f59e0b',
  'post-install': '#3b82f6',
  usb: '#8b5cf6',
};

// ── Component ──────────────────────────────────────────────────────────────

export default function Troubleshoot({ open, onClose }: TroubleshootProps) {
  const { hardware } = useHardware();
  const { buildResult } = useEfi();
  const { report: compatReport } = useCompatibility();

  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [searchQuery, setSearchQuery] = useState('');
  const [activeCategory, setActiveCategory] = useState<string | null>(null);
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [diagSnapshot, setDiagSnapshot] = useState<string>('');
  const [loadingDiag, setLoadingDiag] = useState(false);

  // Build diagnostics snapshot when opening
  useEffect(() => {
    if (!open) return;
    void loadDiagSnapshot();
  }, [open]);

  const loadDiagSnapshot = async () => {
    setLoadingDiag(true);
    try {
      const [version, sessionId, tail] = await Promise.all([
        getVersion(),
        api.logGetSessionId(),
        api.logGetTail(100),
      ]);
      const lines = [
        `OpCore-OneClick v${version}`,
        `Session: ${sessionId}`,
        `Platform: ${navigator.platform}`,
        `Date: ${new Date().toISOString()}`,
        '',
        '--- Hardware ---',
        hardware ? `CPU: ${hardware.cpu.name} (${hardware.cpu.vendor})` : 'No hardware scan',
        hardware ? `GPU: ${hardware.gpu.map(g => g.name).join(', ')}` : '',
        hardware ? `Motherboard: ${hardware.motherboard.manufacturer} ${hardware.motherboard.product}` : '',
        hardware ? `Form factor: ${hardware.isLaptop ? 'Laptop' : 'Desktop'}` : '',
        '',
        '--- Compatibility ---',
        compatReport ? `Verdict: ${compatReport.overall} (${Math.round(compatReport.confidence * 100)}%)` : 'No report',
        compatReport ? `Recommended: ${compatReport.recommendedOs}` : '',
        compatReport?.issues.length ? `Issues: ${compatReport.issues.map(i => `${i.severity}:${i.component}`).join(', ')}` : 'No issues',
        '',
        '--- Build ---',
        buildResult ? `OpenCore: ${buildResult.opencoreVersion}` : 'No build',
        buildResult ? `Kexts: ${buildResult.kexts.map(k => `${k.name}(${k.status})`).join(', ')}` : '',
        buildResult?.warnings.length ? `Warnings: ${buildResult.warnings.join('; ')}` : '',
        '',
        '--- Recent Logs ---',
        tail || '(empty)',
      ].filter(Boolean);
      setDiagSnapshot(lines.join('\n'));
    } catch {
      setDiagSnapshot('Failed to load diagnostics');
    } finally {
      setLoadingDiag(false);
    }
  };

  const handleCopyEntry = useCallback(async (entry: TroubleEntry) => {
    const text = [
      `## ${entry.title}`,
      '',
      '**Symptoms:**',
      ...entry.symptoms.map(s => `- ${s}`),
      '',
      '**Likely Causes:**',
      ...entry.causes.map(c => `- ${c}`),
      '',
      '**Fixes:**',
      ...entry.fixes.map(f => `- ${f}`),
      ...(entry.advanced ? ['', '**Advanced:**', ...entry.advanced.map(a => `- ${a}`)] : []),
      ...(entry.relatedKexts ? ['', `**Kexts:** ${entry.relatedKexts.join(', ')}`] : []),
    ].join('\n');

    await navigator.clipboard.writeText(text);
    setCopiedId(entry.id);
    setTimeout(() => setCopiedId(null), 2000);
  }, []);

  const handleSendReport = useCallback(async () => {
    const title = encodeURIComponent('Bug Report: [describe your issue]');
    const body = encodeURIComponent(
      [
        '## Description',
        '<!-- Describe the issue in detail -->',
        '',
        '## Steps to Reproduce',
        '1. ',
        '2. ',
        '3. ',
        '',
        '## Expected Behavior',
        '',
        '## Diagnostics',
        '```',
        diagSnapshot,
        '```',
      ].join('\n'),
    );
    const url = `https://github.com/redpersongpt/OpCore-OneClick/issues/new?title=${title}&body=${body}&labels=bug`;
    try {
      await openUrl(url);
    } catch {
      window.open(url, '_blank');
    }
  }, [diagSnapshot]);

  // Filter entries
  const filteredEntries = TROUBLE_DB.filter((entry) => {
    if (activeCategory && entry.category !== activeCategory) return false;
    if (!searchQuery) return true;
    const q = searchQuery.toLowerCase();
    return (
      entry.title.toLowerCase().includes(q) ||
      entry.symptoms.some(s => s.toLowerCase().includes(q)) ||
      entry.causes.some(c => c.toLowerCase().includes(q)) ||
      entry.fixes.some(f => f.toLowerCase().includes(q)) ||
      (entry.relatedKexts?.some(k => k.toLowerCase().includes(q)) ?? false)
    );
  });

  const categories = [...new Set(TROUBLE_DB.map(e => e.category))];

  return (
    <Modal
      open={open}
      onClose={onClose}
      title={
        <div className="flex items-center gap-2.5">
          <Bug size={16} className="text-[--color-red-5]" />
          <span>Troubleshoot</span>
        </div>
      }
      width="max-w-4xl"
      noPadding
      footer={
        <div className="flex items-center justify-between w-full">
          <Button
            variant="secondary"
            size="sm"
            onClick={handleSendReport}
            leadingIcon={<AlertTriangle size={13} />}
          >
            Send Bug Report
          </Button>
          <Button variant="secondary" onClick={onClose}>Close</Button>
        </div>
      }
    >
      <div className="flex flex-col max-h-[70vh]">
        {/* Search + Category filters */}
        <div className="px-5 pt-4 pb-3 border-b border-[--border-subtle] space-y-3">
          <div className="relative">
            <Search size={14} className="absolute left-3 top-1/2 -translate-y-1/2 text-[--text-tertiary]" />
            <input
              type="text"
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              placeholder="Search issues, symptoms, kexts..."
              className="w-full h-8 pl-9 pr-3 rounded-md bg-[--surface-1] border border-[--border-subtle] text-[0.8125rem] text-[--text-primary] placeholder:text-[--text-tertiary] focus:outline-none focus:ring-1 focus:ring-[--accent]"
            />
          </div>

          <div className="flex gap-1.5 flex-wrap">
            <button
              onClick={() => setActiveCategory(null)}
              className={`h-6 px-2.5 rounded-full text-[10px] font-medium uppercase tracking-wide transition-colors ${
                !activeCategory
                  ? 'bg-[--accent] text-white'
                  : 'bg-[--surface-1] text-[--text-tertiary] hover:text-[--text-secondary]'
              }`}
            >
              All
            </button>
            {categories.map((cat) => (
              <button
                key={cat}
                onClick={() => setActiveCategory(activeCategory === cat ? null : cat)}
                className={`h-6 px-2.5 rounded-full text-[10px] font-medium uppercase tracking-wide transition-colors ${
                  activeCategory === cat
                    ? 'text-white'
                    : 'bg-[--surface-1] text-[--text-tertiary] hover:text-[--text-secondary]'
                }`}
                style={activeCategory === cat ? { backgroundColor: CATEGORY_COLORS[cat] } : undefined}
              >
                {CATEGORY_LABELS[cat]}
              </button>
            ))}
          </div>
        </div>

        {/* Entries */}
        <div className="flex-1 overflow-y-auto px-5 py-3 space-y-2">
          <AnimatePresence initial={false}>
            {filteredEntries.map((entry) => {
              const isExpanded = expandedId === entry.id;

              return (
                <motion.div
                  key={entry.id}
                  layout
                  initial={{ opacity: 0, y: 4 }}
                  animate={{ opacity: 1, y: 0 }}
                  exit={{ opacity: 0, y: -4 }}
                  className="rounded-lg border border-[--border-subtle] bg-[--surface-1] overflow-hidden"
                >
                  {/* Header */}
                  <button
                    onClick={() => setExpandedId(isExpanded ? null : entry.id)}
                    className="w-full flex items-center gap-3 px-4 py-3 text-left hover:bg-[--surface-2] transition-colors"
                  >
                    <span className="text-[--text-tertiary]">{entry.icon}</span>
                    <span className="flex-1 text-[0.8125rem] font-medium text-[--text-primary]">
                      {entry.title}
                    </span>
                    <span
                      className="text-[9px] uppercase tracking-wider font-semibold px-1.5 py-0.5 rounded"
                      style={{
                        color: CATEGORY_COLORS[entry.category],
                        backgroundColor: `${CATEGORY_COLORS[entry.category]}15`,
                      }}
                    >
                      {CATEGORY_LABELS[entry.category]}
                    </span>
                    {isExpanded ? (
                      <ChevronDown size={14} className="text-[--text-tertiary]" />
                    ) : (
                      <ChevronRight size={14} className="text-[--text-tertiary]" />
                    )}
                  </button>

                  {/* Expanded content */}
                  <AnimatePresence>
                    {isExpanded && (
                      <motion.div
                        initial={{ height: 0, opacity: 0 }}
                        animate={{ height: 'auto', opacity: 1 }}
                        exit={{ height: 0, opacity: 0 }}
                        transition={{ duration: 0.2 }}
                        className="overflow-hidden"
                      >
                        <div className="px-4 pb-4 pt-1 space-y-4 border-t border-[--border-subtle]">
                          {/* Symptoms */}
                          <div>
                            <p className="text-[10px] uppercase tracking-wide text-[--text-tertiary] font-medium mb-1.5">
                              Symptoms
                            </p>
                            <ul className="space-y-1">
                              {entry.symptoms.map((s, i) => (
                                <li key={i} className="flex items-start gap-2 text-[0.75rem] text-[--text-secondary]">
                                  <span className="mt-1.5 w-1 h-1 rounded-full bg-[--text-tertiary] shrink-0" />
                                  {s}
                                </li>
                              ))}
                            </ul>
                          </div>

                          {/* Causes */}
                          <div>
                            <p className="text-[10px] uppercase tracking-wide text-[#f59e0b] font-medium mb-1.5">
                              Likely Causes
                            </p>
                            <ul className="space-y-1">
                              {entry.causes.map((c, i) => (
                                <li key={i} className="flex items-start gap-2 text-[0.75rem] text-[--text-secondary]">
                                  <span className="mt-1.5 w-1 h-1 rounded-full bg-[#f59e0b] shrink-0" />
                                  {c}
                                </li>
                              ))}
                            </ul>
                          </div>

                          {/* Fixes */}
                          <div>
                            <p className="text-[10px] uppercase tracking-wide text-[#22c55e] font-medium mb-1.5">
                              Fixes
                            </p>
                            <ul className="space-y-1.5">
                              {entry.fixes.map((f, i) => (
                                <li key={i} className="flex items-start gap-2 text-[0.75rem] text-[--text-primary]">
                                  <span className="mt-0.5 text-[#22c55e] font-mono text-[10px] shrink-0">{i + 1}.</span>
                                  {f}
                                </li>
                              ))}
                            </ul>
                          </div>

                          {/* Advanced */}
                          {entry.advanced && (
                            <div>
                              <p className="text-[10px] uppercase tracking-wide text-[#8b5cf6] font-medium mb-1.5">
                                Advanced
                              </p>
                              <ul className="space-y-1">
                                {entry.advanced.map((a, i) => (
                                  <li key={i} className="flex items-start gap-2 text-[0.6875rem] text-[--text-tertiary]">
                                    <span className="mt-1.5 w-1 h-1 rounded-full bg-[#8b5cf6] shrink-0" />
                                    {a}
                                  </li>
                                ))}
                              </ul>
                            </div>
                          )}

                          {/* Related kexts */}
                          {entry.relatedKexts && (
                            <div className="flex items-center gap-1.5 flex-wrap">
                              <span className="text-[10px] text-[--text-tertiary] uppercase tracking-wide">Kexts:</span>
                              {entry.relatedKexts.map((k) => (
                                <Badge key={k} variant="info" size="sm">{k}</Badge>
                              ))}
                            </div>
                          )}

                          {/* Copy button */}
                          <div className="flex justify-end">
                            <Button
                              variant="ghost"
                              size="sm"
                              onClick={() => void handleCopyEntry(entry)}
                              leadingIcon={copiedId === entry.id ? <Check size={12} /> : <Copy size={12} />}
                            >
                              {copiedId === entry.id ? 'Copied' : 'Copy'}
                            </Button>
                          </div>
                        </div>
                      </motion.div>
                    )}
                  </AnimatePresence>
                </motion.div>
              );
            })}
          </AnimatePresence>

          {filteredEntries.length === 0 && (
            <div className="py-12 text-center">
              <Search size={24} className="mx-auto text-[--text-tertiary] mb-3" />
              <p className="text-[0.8125rem] text-[--text-tertiary]">No matching issues found.</p>
              <p className="text-[0.6875rem] text-[--text-tertiary] mt-1">
                Try different keywords or{' '}
                <button onClick={handleSendReport} className="text-[--accent] hover:underline">
                  report a new issue
                </button>
              </p>
            </div>
          )}
        </div>

        {/* Diagnostics snapshot (collapsible footer) */}
        <div className="border-t border-[--border-subtle]">
          <details className="group">
            <summary className="px-5 py-2.5 flex items-center gap-2 cursor-pointer text-[0.75rem] text-[--text-tertiary] hover:text-[--text-secondary] select-none">
              <ChevronRight size={12} className="transition-transform group-open:rotate-90" />
              System Diagnostics
              <Button
                variant="ghost"
                size="sm"
                onClick={(e) => { e.preventDefault(); void loadDiagSnapshot(); }}
                leadingIcon={<RefreshCw size={11} className={loadingDiag ? 'animate-spin' : ''} />}
              >
                Refresh
              </Button>
            </summary>
            <div className="px-5 pb-3 max-h-[200px] overflow-auto">
              <pre className="whitespace-pre-wrap break-words font-mono text-[0.625rem] leading-4 text-[--text-tertiary]">
                {diagSnapshot || 'Loading...'}
              </pre>
            </div>
          </details>
        </div>
      </div>
    </Modal>
  );
}
