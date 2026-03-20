import React, { useState, useEffect } from 'react';
import { motion, AnimatePresence } from 'motion/react';
import {
  CheckCircle2, XCircle, AlertTriangle, Loader2, RefreshCcw,
  Shield, HardDrive, Wifi, Usb, Cpu, Monitor, Wrench,
} from 'lucide-react';
import { STEP_LOAD_TIMEOUT_MS } from '../../config';
import { REMEDIATION_GUIDE } from '../../lib/remediations';

// ── Types ─────────────────────────────────────────────────────────────────────

type CheckStatus = 'pending' | 'running' | 'pass' | 'warn' | 'block';

interface PrecheckItem {
  id: string;
  icon: React.ElementType;
  title: string;
  description: string;
  status: CheckStatus;
  detail: string | null;
  howToFix: string | null;
  /** Group this item belongs to */
  group: string;
}

interface PrecheckResult {
  platform: string;
  adminPrivileges: boolean;
  /** Platform-specific note for the admin check — present on macOS. */
  adminNote?: string | null;
  freeSpaceMB: number;
  networkOk: boolean;
  usbDetected: boolean;
  firmwareDetectionAvailable: boolean;
  missingBinaries: string[];
}

interface Props {
  onContinue: () => void;
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function formatGB(mb: number): string {
  if (mb === Infinity || mb > 999_999) return '(unlimited)';
  return `${(mb / 1024).toFixed(1)} GB`;
}

function buildItems(result: PrecheckResult): PrecheckItem[] {
  const SUPPORTED_PLATFORMS = ['win32', 'darwin', 'linux'];
  const hostSupported = SUPPORTED_PLATFORMS.includes(result.platform);

  const platformLabel: Record<string, string> = {
    win32: 'Windows',
    darwin: 'macOS',
    linux: 'Linux',
  };
  const platformName = platformLabel[result.platform] ?? result.platform;

  let diskStatus: CheckStatus;
  let diskDetail: string;
  let diskFix: string | null = null;
  if (result.freeSpaceMB < 2048) {
    diskStatus = 'block';
    diskDetail = `Only ${formatGB(result.freeSpaceMB)} free — need at least 2 GB to proceed.`;
    diskFix = REMEDIATION_GUIDE.LOW_DISK_SPACE.howToFix;
  } else if (result.freeSpaceMB < 8192) {
    diskStatus = 'warn';
    diskDetail = `${formatGB(result.freeSpaceMB)} free — 8 GB or more recommended for the full download.`;
    diskFix = REMEDIATION_GUIDE.LOW_DISK_SPACE.howToFix;
  } else {
    diskStatus = 'pass';
    diskDetail = `${formatGB(result.freeSpaceMB)} free on host machine.`;
  }

  const items: PrecheckItem[] = [
    // Environment
    {
      id: 'host-os',
      icon: Monitor,
      title: 'Host operating system',
      description: 'Windows, Linux, and macOS hosts are supported.',
      status: hostSupported ? 'pass' : 'warn',
      detail: hostSupported
        ? `${platformName} — supported.`
        : `${platformName} is not a fully tested platform. Some features may not work.`,
      howToFix: hostSupported ? null : REMEDIATION_GUIDE.UNSUPPORTED_HOST.howToFix,
      group: 'Environment',
    },
    // Permissions
    {
      id: 'admin',
      icon: Shield,
      title: 'Administrator / write permissions',
      description: 'Disk write operations require elevated privileges.',
      status: result.adminPrivileges ? 'pass' : 'block',
      detail: result.adminNote
        ? result.adminNote
        : result.adminPrivileges
        ? 'Running with sufficient permissions.'
        : 'Insufficient permissions — cannot write to disks.',
      howToFix: result.adminPrivileges ? null : REMEDIATION_GUIDE.NO_ADMIN.howToFix,
      group: 'Permissions',
    },
    // Storage
    {
      id: 'disk-space',
      icon: HardDrive,
      title: 'Free disk space on this machine',
      description: 'At least 8 GB recommended for the recovery download and EFI build.',
      status: diskStatus,
      detail: diskDetail,
      howToFix: diskFix,
      group: 'Storage',
    },
    // Connectivity
    {
      id: 'network',
      icon: Wifi,
      title: 'Network connection',
      description: 'Internet access is required to download macOS recovery files.',
      status: result.networkOk ? 'pass' : 'warn',
      detail: result.networkOk
        ? 'Internet connection detected (Apple servers reachable).'
        : 'Could not reach Apple servers. The macOS download will fail without a connection.',
      howToFix: result.networkOk ? null : REMEDIATION_GUIDE.NETWORK_UNAVAILABLE.howToFix,
      group: 'Connectivity',
    },
    // Device Readiness
    {
      id: 'usb',
      icon: Usb,
      title: 'Removable USB drive',
      description: 'A removable USB drive (≥ 16 GB) must be connected.',
      status: result.usbDetected ? 'pass' : 'warn',
      detail: result.usbDetected
        ? 'At least one removable USB drive found.'
        : 'No removable USB drives detected. You can connect one later, but it is needed before flashing.',
      howToFix: result.usbDetected ? null : REMEDIATION_GUIDE.NO_USB_DETECTED.howToFix,
      group: 'Device Readiness',
    },
    {
      id: 'firmware-detect',
      icon: Cpu,
      title: 'Firmware detection',
      description: 'Automatic BIOS/UEFI probing is not available on macOS hosts.',
      status: result.firmwareDetectionAvailable ? 'pass' : 'warn',
      detail: result.firmwareDetectionAvailable
        ? 'Firmware detection is available on this platform.'
        : 'Firmware detection is not available when running on a Mac. Verify BIOS/UEFI settings manually on the target PC.',
      howToFix: result.firmwareDetectionAvailable ? null : REMEDIATION_GUIDE.FIRMWARE_UNVERIFIED.howToFix,
      group: 'Device Readiness',
    },
    // Tools
    {
      id: 'binaries',
      icon: Wrench,
      title: 'Required system tools',
      description: 'Platform utilities (diskutil, diskpart, parted, etc.) must be present.',
      status:
        result.missingBinaries.length === 0
          ? 'pass'
          : 'block',
      detail:
        result.missingBinaries.length === 0
          ? 'All required system tools are present.'
          : `Missing: ${result.missingBinaries.join(', ')}`,
      howToFix:
        result.missingBinaries.length === 0
          ? null
          : REMEDIATION_GUIDE.MISSING_BINARY.howToFix,
      group: 'Tools',
    },
  ];

  return items;
}

// Group ordering
const GROUP_ORDER = ['Environment', 'Permissions', 'Storage', 'Connectivity', 'Device Readiness', 'Tools'];

// ── Status display config ──────────────────────────────────────────────────────

const STATUS_CONFIG: Record<CheckStatus, {
  icon: React.ElementType;
  iconClass: string;
  rowBg: string;
  rowBorder: string;
  badgeLabel: string;
  badgeClass: string;
}> = {
  pending: {
    icon: () => <div className="w-3 h-3 rounded-full bg-white/15" />,
    iconClass: 'text-white/30',
    rowBg: 'bg-transparent',
    rowBorder: 'border-transparent',
    badgeLabel: 'Pending',
    badgeClass: 'bg-white/5 border-white/10 text-white/25',
  },
  running: {
    icon: Loader2,
    iconClass: 'text-blue-400 animate-spin',
    rowBg: 'bg-blue-500/4',
    rowBorder: 'border-blue-500/15',
    badgeLabel: 'Checking…',
    badgeClass: 'bg-blue-500/10 border-blue-500/20 text-blue-400',
  },
  pass: {
    icon: CheckCircle2,
    iconClass: 'text-emerald-400',
    rowBg: 'bg-emerald-500/4',
    rowBorder: 'border-emerald-500/15',
    badgeLabel: 'Pass',
    badgeClass: 'bg-emerald-500/10 border-emerald-500/20 text-emerald-400',
  },
  warn: {
    icon: AlertTriangle,
    iconClass: 'text-amber-400',
    rowBg: 'bg-amber-500/4',
    rowBorder: 'border-amber-500/15',
    badgeLabel: 'Warning',
    badgeClass: 'bg-amber-500/10 border-amber-500/20 text-amber-400',
  },
  block: {
    icon: XCircle,
    iconClass: 'text-red-400',
    rowBg: 'bg-red-500/4',
    rowBorder: 'border-red-500/15',
    badgeLabel: 'Blocked',
    badgeClass: 'bg-red-500/10 border-red-500/20 text-red-400',
  },
};

// ── Row component ─────────────────────────────────────────────────────────────

function PrecheckRow({ item }: { item: PrecheckItem }) {
  const [expanded, setExpanded] = useState(false);
  const cfg = STATUS_CONFIG[item.status];
  const Icon = cfg.icon;
  const ItemIcon = item.icon;

  const isActionable = item.status === 'warn' || item.status === 'block';

  return (
    <motion.div
      layout
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      className={`rounded-2xl border overflow-hidden transition-all ${cfg.rowBg} ${cfg.rowBorder}`}
    >
      <div
        className={`flex items-center gap-4 px-5 py-4 ${isActionable ? 'cursor-pointer hover:bg-white/3 transition-colors' : ''}`}
        onClick={() => isActionable && setExpanded(x => !x)}
      >
        {/* Item icon */}
        <div className={`w-8 h-8 rounded-xl flex items-center justify-center flex-shrink-0 ${
          item.status === 'pass' ? 'bg-emerald-500/10' :
          item.status === 'block' ? 'bg-red-500/10' :
          item.status === 'warn' ? 'bg-amber-500/10' :
          'bg-white/6'
        }`}>
          <ItemIcon className={`w-4 h-4 ${
            item.status === 'pass' ? 'text-emerald-400' :
            item.status === 'block' ? 'text-red-400' :
            item.status === 'warn' ? 'text-amber-400' :
            'text-white/30'
          }`} />
        </div>

        {/* Title + description */}
        <div className="flex-1 min-w-0">
          <div className={`text-sm font-bold ${
            item.status === 'block' ? 'text-red-300' :
            item.status === 'pass' ? 'text-white/80' :
            item.status === 'warn' ? 'text-amber-200/80' :
            'text-white/40'
          }`}>{item.title}</div>
          {item.status === 'running' ? (
            <div className="text-[10px] text-white/30 mt-0.5">Checking…</div>
          ) : item.detail ? (
            <div className="text-[10px] text-white/40 mt-0.5 leading-snug">{item.detail}</div>
          ) : (
            <div className="text-[10px] text-white/25 mt-0.5">{item.description}</div>
          )}
        </div>

        {/* Status badge */}
        <div className={`flex-shrink-0 px-2 py-0.5 rounded-full text-[9px] font-bold border ${cfg.badgeClass}`}>
          {cfg.badgeLabel}
        </div>

        {/* Status icon */}
        <Icon className={`w-4 h-4 flex-shrink-0 ${cfg.iconClass}`} />
      </div>

      {/* Expandable how-to-fix panel */}
      <AnimatePresence>
        {expanded && item.howToFix && (
          <motion.div
            initial={{ height: 0 }} animate={{ height: 'auto' }} exit={{ height: 0 }}
            className="overflow-hidden"
          >
            <div className="px-5 pb-4 pt-2 border-t border-white/5 flex items-start gap-3">
              <div className="text-[9px] font-bold uppercase tracking-wider text-white/25 pt-1 w-20 flex-shrink-0">
                How to fix
              </div>
              <div className="text-xs text-white/50 leading-relaxed">{item.howToFix}</div>
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </motion.div>
  );
}

// ── Main component ────────────────────────────────────────────────────────────

export default function PrecheckStep({ onContinue }: Props) {
  const [loadState, setLoadState] = useState<'running' | 'done' | 'timeout' | 'error'>('running');
  const [items, setItems] = useState<PrecheckItem[]>([]);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  const runChecks = async () => {
    setLoadState('running');
    setErrorMsg(null);

    // Create placeholder pending items while loading
    setItems([
      { id: 'host-os',         icon: Monitor,   title: 'Host operating system',       description: '', status: 'running', detail: null, howToFix: null, group: 'Environment' },
      { id: 'admin',           icon: Shield,    title: 'Administrator permissions',   description: '', status: 'running', detail: null, howToFix: null, group: 'Permissions' },
      { id: 'disk-space',      icon: HardDrive, title: 'Free disk space',             description: '', status: 'running', detail: null, howToFix: null, group: 'Storage' },
      { id: 'network',         icon: Wifi,      title: 'Network connection',          description: '', status: 'running', detail: null, howToFix: null, group: 'Connectivity' },
      { id: 'usb',             icon: Usb,       title: 'Removable USB drive',         description: '', status: 'running', detail: null, howToFix: null, group: 'Device Readiness' },
      { id: 'firmware-detect', icon: Cpu,       title: 'Firmware detection',          description: '', status: 'running', detail: null, howToFix: null, group: 'Device Readiness' },
      { id: 'binaries',        icon: Wrench,    title: 'Required system tools',       description: '', status: 'running', detail: null, howToFix: null, group: 'Tools' },
    ]);

    try {
      const result = await Promise.race([
        window.electron.runPrechecks(),
        new Promise<never>((_, reject) =>
          setTimeout(() => reject(new Error('PRECHECK_TIMEOUT')), STEP_LOAD_TIMEOUT_MS)
        ),
      ]);
      setItems(buildItems(result));
      setLoadState('done');
    } catch (e: any) {
      if (e?.message === 'PRECHECK_TIMEOUT') {
        setLoadState('timeout');
        setItems(prev => prev.map(item => ({
          ...item,
          status: 'warn' as CheckStatus,
          detail: 'Check timed out — could not determine status.',
          howToFix: REMEDIATION_GUIDE.PRECHECK_TIMEOUT.howToFix,
        })));
      } else {
        setLoadState('error');
        setErrorMsg(e?.message ?? 'Unknown error during system checks.');
      }
    }
  };

  useEffect(() => {
    runChecks();
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const hasBlock = items.some(i => i.status === 'block');
  const hasWarn  = items.some(i => i.status === 'warn');

  const canContinue = loadState === 'done' && !hasBlock;

  // Group items
  const groupedItems = GROUP_ORDER.map(groupName => ({
    groupName,
    items: items.filter(item => item.group === groupName),
  })).filter(g => g.items.length > 0);

  return (
    <div className="flex flex-col h-full space-y-5">
      {/* Header */}
      <div className="flex-shrink-0 flex items-start justify-between">
        <div>
          <h2 className="text-4xl font-bold text-white mb-2">System Check</h2>
          <p className="text-[#888888] text-sm font-medium">
            Checking that your system meets the requirements.
          </p>
        </div>
        {loadState !== 'running' && (
          <button
            onClick={runChecks}
            className="flex items-center gap-2 px-4 py-2 bg-white/5 border border-white/10 rounded-xl text-xs font-bold text-white/60 hover:text-white hover:bg-white/8 transition-all cursor-pointer flex-shrink-0"
          >
            <RefreshCcw className="w-3.5 h-3.5" /> Re-run
          </button>
        )}
      </div>

      {/* Top summary banner */}
      <AnimatePresence>
        {loadState === 'done' && (
          <motion.div
            initial={{ opacity: 0, y: 6 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0 }}
            className={`flex-shrink-0 flex items-center gap-3 px-4 py-3 rounded-2xl border ${
              hasBlock
                ? 'bg-red-500/8 border-red-500/20'
                : hasWarn
                ? 'bg-amber-500/8 border-amber-500/20'
                : 'bg-emerald-500/6 border-emerald-500/20'
            }`}
          >
            {hasBlock ? (
              <XCircle className="w-4 h-4 text-red-400 flex-shrink-0" />
            ) : hasWarn ? (
              <AlertTriangle className="w-4 h-4 text-amber-400 flex-shrink-0" />
            ) : (
              <CheckCircle2 className="w-4 h-4 text-emerald-400 flex-shrink-0" />
            )}
            <span className={`text-xs font-medium leading-relaxed ${
              hasBlock ? 'text-red-300/80' : hasWarn ? 'text-amber-300/80' : 'text-emerald-300/80'
            }`}>
              {hasBlock
                ? 'Some requirements are not met. Fix the blocked items below.'
                : hasWarn
                ? 'Warnings found — you can continue, but review them first.'
                : 'All checks passed. Ready to continue.'}
            </span>
          </motion.div>
        )}
        {loadState === 'timeout' && (
          <motion.div initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0 }}
            className="flex-shrink-0 flex items-start gap-3 px-4 py-3 rounded-2xl bg-amber-500/8 border border-amber-500/20">
            <AlertTriangle className="w-4 h-4 text-amber-400 flex-shrink-0 mt-0.5" />
            <div className="text-xs text-amber-300/80 leading-relaxed">
              <span className="font-bold">System check timed out.</span>{' '}
              One or more checks did not return a result within {STEP_LOAD_TIMEOUT_MS / 1000} seconds.
              Click Re-run to try again.
            </div>
          </motion.div>
        )}
        {loadState === 'error' && errorMsg && (
          <motion.div initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0 }}
            className="flex-shrink-0 flex items-start gap-3 px-4 py-3 rounded-2xl bg-red-500/8 border border-red-500/20">
            <XCircle className="w-4 h-4 text-red-400 flex-shrink-0 mt-0.5" />
            <div className="text-xs text-red-300/80 leading-relaxed">
              <span className="font-bold">Check failed: </span>{errorMsg}
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* Grouped check list */}
      <div className="flex-1 overflow-y-auto custom-scrollbar space-y-5 pr-1">
        {loadState === 'running' && groupedItems.length === 0 ? (
          // Show flat running list while waiting for groups to populate
          <div className="space-y-2">
            {items.map((item, i) => (
              <motion.div key={item.id} initial={{ opacity: 0, x: 10 }} animate={{ opacity: 1, x: 0 }} transition={{ delay: i * 0.04 }}>
                <PrecheckRow item={item} />
              </motion.div>
            ))}
          </div>
        ) : (
          groupedItems.map((group, gi) => (
            <div key={group.groupName} className="space-y-2">
              <div className="text-[9px] font-bold uppercase tracking-widest text-white/20 px-1">
                {group.groupName}
              </div>
              <AnimatePresence mode="popLayout">
                {group.items.map((item, i) => (
                  <motion.div
                    key={item.id}
                    initial={{ opacity: 0, x: 10 }}
                    animate={{ opacity: 1, x: 0 }}
                    transition={{ delay: (gi * 0.05) + (i * 0.03), type: 'spring', stiffness: 260, damping: 20 }}
                  >
                    <PrecheckRow item={item} />
                  </motion.div>
                ))}
              </AnimatePresence>
            </div>
          ))
        )}
      </div>

      {/* Footer */}
      <div className="flex-shrink-0 pt-4 border-t border-white/5 flex items-center justify-between">
        <div className="text-xs text-[#555555]">
          {loadState === 'running' ? (
            <span className="flex items-center gap-1.5">
              <Loader2 className="w-3 h-3 animate-spin text-blue-400" /> Checking…
            </span>
          ) : hasBlock ? (
            <span className="text-red-400/70">Fix blocked items to continue.</span>
          ) : hasWarn ? (
            <span className="text-amber-400/60">Warnings present — review before continuing.</span>
          ) : (
            <span className="text-emerald-400/60">All checks passed.</span>
          )}
        </div>
        <button
          onClick={onContinue}
          disabled={!canContinue}
          title={hasBlock ? 'One or more required checks failed. Fix the blocked items before continuing.' : undefined}
          className={`px-8 py-3 rounded-xl text-sm font-bold transition-all ${
            canContinue
              ? 'bg-blue-600 text-white hover:bg-blue-500 cursor-pointer shadow-lg shadow-blue-600/20'
              : 'bg-white/5 text-white/30 cursor-not-allowed'
          }`}
        >
          {hasBlock
            ? 'Fix the items above to continue'
            : canContinue
              ? hasWarn
                ? 'Continue with warnings →'
                : 'Continue →'
              : loadState === 'running'
              ? 'Checking…'
              : 'Continue →'}
        </button>
      </div>
    </div>
  );
}
