import React, { useState } from 'react';
import { motion, AnimatePresence } from 'motion/react';
import {
  Usb, ShieldAlert, RefreshCcw, ChevronDown, HelpCircle,
  CheckCircle2, AlertTriangle, XCircle, HardDrive, Info, Eye,
} from 'lucide-react';
import { REMEDIATION_GUIDE } from '../../lib/remediations';
import { BEGINNER_SAFETY_MODE } from '../../config';
import type { EfiBackupPolicy } from '../../../electron/efiBackup';
import EfiBackupPanel from '../EfiBackupPanel';

// ── Types ─────────────────────────────────────────────────────────────────────

export interface DriveInfo {
  name: string;
  device: string;
  size: string;
  /** Populated when getDiskInfo has been called for this device. */
  isSystemDisk?: boolean;
  partitionTable?: 'gpt' | 'mbr' | 'unknown';
  mountedPartitions?: string[];
  removable?: boolean;
}

/** Classification tier for a drive in the selection list. */
type DriveTier = 'safe' | 'suspicious' | 'blocked';

interface BlockReason {
  code: string;
  label: string;
  explanation: string;
  howToFix: string | null;
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function classifyDrive(drive: DriveInfo, requireFullSize: boolean): { tier: DriveTier; reasons: BlockReason[]; unreliable?: boolean } {
  const reasons: BlockReason[] = [];
  let unreliable = false;

  const sizeGB = parseSizeGB(drive.size);
  
  // Unreliable markers
  const nameLower = drive.name.toLowerCase();
  if (nameLower.includes('generic') || nameLower.includes('innostor') || nameLower.includes('mass storage')) {
    unreliable = true;
  }

  if (requireFullSize && sizeGB > 0 && sizeGB < 14.5) {
    reasons.push({
      code: 'DEVICE_TOO_SMALL',
      label: 'Drive too small',
      explanation: REMEDIATION_GUIDE.DEVICE_TOO_SMALL.explanation,
      howToFix: REMEDIATION_GUIDE.DEVICE_TOO_SMALL.howToFix,
    });
  }

  if (drive.isSystemDisk === true) {
    reasons.push({
      code: 'SYSTEM_DISK',
      label: 'Main system drive',
      explanation: REMEDIATION_GUIDE.SYSTEM_DISK.explanation,
      howToFix: REMEDIATION_GUIDE.SYSTEM_DISK.howToFix,
    });
  }

  if (drive.removable === false) {
    reasons.push({
      code: 'NOT_REMOVABLE',
      label: 'Internal drive',
      explanation: REMEDIATION_GUIDE.NOT_REMOVABLE.explanation,
      howToFix: REMEDIATION_GUIDE.NOT_REMOVABLE.howToFix,
    });
  }

  if (drive.partitionTable === 'unknown') {
    reasons.push({
      code: 'UNKNOWN_PARTITION_TABLE',
      label: 'Unreadable partition table',
      explanation: REMEDIATION_GUIDE.UNKNOWN_PARTITION_TABLE.explanation,
      howToFix: REMEDIATION_GUIDE.UNKNOWN_PARTITION_TABLE.howToFix,
    });
  }

  if (drive.partitionTable === 'mbr') {
    reasons.push({
      code: 'MBR_PARTITION_TABLE',
      label: 'MBR partition table',
      explanation: REMEDIATION_GUIDE.MBR_PARTITION_TABLE.explanation,
      howToFix: REMEDIATION_GUIDE.MBR_PARTITION_TABLE.howToFix,
    });
  }

  // Hard block: system disk, confirmed non-removable, MBR, or too small
  const hardBlock = reasons.some(r => r.code === 'SYSTEM_DISK' || r.code === 'NOT_REMOVABLE' || r.code === 'MBR_PARTITION_TABLE' || r.code === 'DEVICE_TOO_SMALL');
  if (hardBlock) return { tier: 'blocked', reasons, unreliable };

  // Disk info not yet loaded — show as suspicious until confirmed
  if (drive.isSystemDisk === undefined || drive.partitionTable === undefined) {
    return {
      tier: 'suspicious',
      unreliable: true,
      reasons: [{
        code: 'UNVERIFIED',
        label: 'Not yet verified',
        explanation: 'Drive safety has not been confirmed yet. Select the drive to verify it.',
        howToFix: null,
      }],
    };
  }

  // Unknown partition table without other hard blocks → suspicious
  if (reasons.some(r => r.code === 'UNKNOWN_PARTITION_TABLE')) {
    return { tier: 'suspicious', reasons, unreliable: true };
  }

  return { tier: 'safe', reasons: [], unreliable };
}

/** Format a partition table value for display. */
function formatPartitionTable(pt: 'gpt' | 'mbr' | 'unknown' | undefined): string {
  if (pt === 'gpt') return 'GPT';
  if (pt === 'mbr') return 'MBR';
  if (pt === 'unknown') return 'Unknown';
  return '—';
}

function parseSizeGB(sizeStr: string): number {
  const match = sizeStr.match(/([\d.]+)\s*([KMGT]i?B?|B)?/i);
  if (!match) return 0;
  const val = parseFloat(match[1]);
  const unit = (match[2] || '').toUpperCase();
  if (unit.startsWith('T')) return val * 1000;
  if (unit.startsWith('G')) return val;
  if (unit.startsWith('M')) return val / 1000;
  if (unit.startsWith('K')) return val / 1e6;
  return val / 1e9;
}

// ── Tooltip component ─────────────────────────────────────────────────────────

function JargonTooltip({ term, definition }: { term: string; definition: string }) {
  const [open, setOpen] = useState(false);
  return (
    <span className="relative inline-flex items-baseline gap-0.5">
      <span>{term}</span>
      <button
        onClick={e => { e.stopPropagation(); setOpen(x => !x); }}
        className="inline-flex items-center cursor-pointer text-white/25 hover:text-white/50 transition-colors"
      >
        <HelpCircle className="w-2.5 h-2.5 ml-0.5 align-baseline" />
      </button>
      <AnimatePresence>
        {open && (
          <motion.div
            initial={{ opacity: 0, y: 4 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: 4 }}
            className="absolute bottom-full left-0 z-20 mb-2 w-52 px-3 py-2 bg-[#111] border border-white/12 rounded-xl text-xs text-white/60 leading-relaxed shadow-xl"
          >
            {definition}
            <button
              onClick={e => { e.stopPropagation(); setOpen(false); }}
              className="absolute top-1.5 right-2 text-white/30 hover:text-white/60 cursor-pointer"
            >
              ×
            </button>
          </motion.div>
        )}
      </AnimatePresence>
    </span>
  );
}

// ── Device identity panel ─────────────────────────────────────────────────────

interface DeviceIdentityPanelProps {
  drive: DriveInfo;
  onRescan: () => void;
  driveStillPresent: boolean;
}

function DeviceIdentityPanel({ drive, onRescan, driveStillPresent }: DeviceIdentityPanelProps) {
  const rows: { label: string; value: string; tooltip?: string }[] = [
    { label: 'Disk identifier', value: drive.device, tooltip: 'The operating system\'s internal name for this drive. You will type this to confirm flashing.' },
    { label: 'Size', value: drive.size },
    {
      label: 'Removable',
      value: drive.removable === true ? 'Yes (USB/External)' : drive.removable === false ? 'No (Internal)' : 'Unknown',
      tooltip: 'Whether the OS reports this drive as removable/external. Internal drives cannot be flashed.',
    },
    {
      label: 'Partition table',
      value: formatPartitionTable(drive.partitionTable),
      tooltip: 'GPT (GUID Partition Table) is required for OpenCore. MBR must be converted first. Unknown means the table could not be read.',
    },
    {
      label: 'System drive',
      value: drive.isSystemDisk === true ? 'YES — cannot flash' : drive.isSystemDisk === false ? 'No' : 'Unknown',
    },
    {
      label: 'Mount points',
      value: drive.mountedPartitions && drive.mountedPartitions.length > 0
        ? drive.mountedPartitions.join(', ')
        : '—',
      tooltip: 'Volumes currently mounted from this drive. Mounted volumes indicate the drive is in use.',
    },
  ];

  return (
    <motion.div
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      className="rounded-2xl border border-white/8 bg-white/3 overflow-hidden"
    >
      <div className="px-4 py-2.5 border-b border-white/5 flex items-center justify-between">
        <span className="text-[9px] font-bold uppercase tracking-widest text-white/25">
          Drive details
        </span>
        <button
          onClick={onRescan}
          className="flex items-center gap-1.5 text-[10px] text-white/30 hover:text-white/60 transition-colors cursor-pointer"
        >
          <RefreshCcw className="w-3 h-3" /> Re-scan now
        </button>
      </div>
      <div className="divide-y divide-white/4">
        {rows.map(({ label, value, tooltip }) => (
          <div key={label} className="flex items-center justify-between px-4 py-2.5">
            <span className="text-xs text-white/35 w-36 flex-shrink-0">
              {tooltip ? (
                <JargonTooltip term={label} definition={tooltip} />
              ) : label}
            </span>
            <span className="text-xs font-mono text-white/70 text-right min-w-0 truncate">{value}</span>
          </div>
        ))}
      </div>

      {/* Identifier instability notice */}
      <div className="px-4 py-3 border-t border-amber-500/15 bg-amber-500/4">
        <div className="flex items-start gap-2">
          <AlertTriangle className="w-3.5 h-3.5 text-amber-400 flex-shrink-0 mt-0.5" />
          <p className="text-[10px] text-amber-300/70 leading-relaxed">
            <span className="font-bold text-amber-300/90">Important:</span>{' '}
            If you disconnect and reconnect this drive, its identifier may change.
            Re-check the identifier before confirming the flash.
          </p>
        </div>
      </div>

      {/* Drive no longer detected warning */}
      {!driveStillPresent && (
        <motion.div
          initial={{ opacity: 0, y: 4 }}
          animate={{ opacity: 1, y: 0 }}
          className="px-4 py-3 border-t border-red-500/20 bg-red-500/8"
        >
          <div className="flex items-start gap-2">
            <XCircle className="w-3.5 h-3.5 text-red-400 flex-shrink-0 mt-0.5" />
            <p className="text-xs text-red-300/80 leading-relaxed font-medium">
              The previously selected drive is no longer detected. Please select again.
            </p>
          </div>
        </motion.div>
      )}
    </motion.div>
  );
}

// ── Drive card ────────────────────────────────────────────────────────────────

function DriveCard({
  drive,
  selected,
  onSelect,
  beginnerBlocked,
  advancedAckGranted,
  requireFullSize,
}: {
  drive: DriveInfo;
  selected: boolean;
  onSelect: (dev: string) => void;
  beginnerBlocked: boolean;
  advancedAckGranted: boolean;
  requireFullSize: boolean;
  key?: string;
}) {
  const [reasonExpanded, setReasonExpanded] = useState(false);
  const { tier, reasons } = classifyDrive(drive, requireFullSize);

  // In beginner mode: blocked tier is always blocked; suspicious tier is blocked
  // unless the user has granted typed acknowledgement for this specific drive.
  const isBlocked = tier === 'blocked' || (beginnerBlocked && !advancedAckGranted);
  const isSuspicious = tier === 'suspicious';

  const tierStyles = {
    safe: {
      card: selected
        ? 'bg-blue-500/8 border-blue-500/30'
        : 'bg-white/3 border-white/6 hover:bg-white/6 hover:border-white/12',
      badge: 'bg-emerald-500/10 border-emerald-500/20 text-emerald-400',
      badgeLabel: 'Safe',
      icon: CheckCircle2,
      iconClass: selected ? 'text-blue-400' : 'text-emerald-400',
    },
    suspicious: {
      card: isBlocked
        ? 'bg-amber-500/3 border-amber-500/10 opacity-50 cursor-not-allowed'
        : selected
        ? 'bg-amber-500/8 border-amber-500/30'
        : 'bg-amber-500/3 border-amber-500/15 hover:bg-amber-500/6 hover:border-amber-500/25',
      badge: 'bg-amber-500/10 border-amber-500/20 text-amber-400',
      badgeLabel: 'Suspicious',
      icon: AlertTriangle,
      iconClass: 'text-amber-400',
    },
    blocked: {
      card: 'bg-red-500/4 border-red-500/15 opacity-60 cursor-not-allowed',
      badge: 'bg-red-500/10 border-red-500/20 text-red-400',
      badgeLabel: 'Blocked',
      icon: XCircle,
      iconClass: 'text-red-400',
    },
  };

  const style = tierStyles[tier];
  const TierIcon = style.icon;

  // If suspicious and beginner-blocked (not in advanced), dim the card
  const cardClass = (isSuspicious && beginnerBlocked && !advancedAckGranted)
    ? 'bg-amber-500/3 border-amber-500/10 opacity-50 cursor-not-allowed'
    : style.card;

  return (
    <div>
      <motion.button
        initial={{ opacity: 0, y: 10 }}
        animate={{ opacity: 1, y: 0 }}
        whileHover={isBlocked ? undefined : { scale: 1.01 }}
        whileTap={isBlocked ? undefined : { scale: 0.98 }}
        onClick={() => !isBlocked && onSelect(drive.device)}
        disabled={isBlocked}
        className={`magnetic-glow w-full flex items-center gap-5 p-5 rounded-3xl border text-left transition-all ${cardClass}`}
      >
        {/* Drive icon */}
        <motion.div
          animate={{
            scale: selected ? 1.05 : 1,
            backgroundColor: selected ? 'rgba(59, 130, 246, 0.2)' : 'rgba(255, 255, 255, 0.06)',
          }}
          className="w-14 h-14 rounded-2xl flex items-center justify-center flex-shrink-0 transition-all"
        >
          {tier === 'blocked' ? (
            <HardDrive className="w-7 h-7 text-red-400/60" />
          ) : isSuspicious && beginnerBlocked && !advancedAckGranted ? (
            <HardDrive className="w-7 h-7 text-amber-400/40" />
          ) : (
            <Usb className={`w-7 h-7 ${selected ? 'text-blue-400' : 'text-[#888]'}`} />
          )}
        </motion.div>

        {/* Name + device */}
        <div className="flex-1 min-w-0">
          <div className="text-base font-bold text-white truncate">{drive.name}</div>
          <div className="text-xs font-mono text-white/20 mt-0.5 tracking-tight">{drive.device}</div>
        </div>

        {/* Size + status */}
        <div className="text-right flex-shrink-0 space-y-1">
          <div className="text-lg font-bold text-white tracking-tight">{drive.size}</div>
          <div className={`inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[9px] font-bold border ${style.badge}`}>
            <TierIcon className="w-2.5 h-2.5" />
            {style.badgeLabel}
          </div>
          {selected && !isBlocked && (
            <motion.div
              initial={{ opacity: 0, scale: 0.9 }}
              animate={{ opacity: 1, scale: 1 }}
              className="text-[10px] font-bold text-blue-400 uppercase tracking-widest"
            >
              Target Drive
            </motion.div>
          )}
        </div>
      </motion.button>

      {/* Block / suspicious reason — expandable inline */}
      {(tier === 'blocked' || tier === 'suspicious') && reasons.length > 0 && (
        <div className="mt-1 mx-2">
          <button
            onClick={() => setReasonExpanded(x => !x)}
            className="flex items-center gap-1.5 text-[10px] text-white/30 hover:text-white/50 transition-colors cursor-pointer px-3 py-1"
          >
            <Info className="w-3 h-3" />
            {tier === 'blocked' ? `Blocked: ${reasons[0].label}` : `Warning: ${reasons[0].label}`}
            <motion.span animate={{ rotate: reasonExpanded ? 180 : 0 }}>
              <ChevronDown className="w-3 h-3" />
            </motion.span>
          </button>
          <AnimatePresence>
            {reasonExpanded && (
              <motion.div
                initial={{ height: 0, opacity: 0 }}
                animate={{ height: 'auto', opacity: 1 }}
                exit={{ height: 0, opacity: 0 }}
                className="overflow-hidden"
              >
                <div className="px-3 pb-3 space-y-2">
                  {reasons.map(reason => (
                    <div key={reason.code} className="text-xs text-white/45 leading-relaxed">
                      <span className="font-bold text-white/60">{reason.label}: </span>
                      {reason.explanation}
                      {reason.howToFix && (
                        <div className="mt-1.5 text-[10px] text-white/30">
                          <span className="font-bold text-white/40">How to fix: </span>
                          {reason.howToFix}
                        </div>
                      )}
                    </div>
                  ))}
                </div>
              </motion.div>
            )}
          </AnimatePresence>
        </div>
      )}
    </div>
  );
}

// ── Advanced device acknowledgement panel ─────────────────────────────────────

const SUSPICIOUS_ACK_TEXT = 'I understand this drive has not been fully identified';

interface AdvancedDriveRowProps {
  drive: DriveInfo;
  selected: boolean;
  onSelect: (dev: string) => void;
  ackGranted: boolean;
  onAckGranted: () => void;
  requireFullSize: boolean;
  key?: string;
}

function AdvancedDriveRow({ drive, selected, onSelect, ackGranted, onAckGranted, requireFullSize }: AdvancedDriveRowProps) {
  const [ackText, setAckText] = useState('');
  const ackValid = ackText.trim() === SUSPICIOUS_ACK_TEXT;

  return (
    <div className="rounded-2xl border border-amber-500/20 bg-amber-500/4 overflow-hidden">
      {/* Warning header */}
      <div className="px-4 py-3 border-b border-amber-500/15 flex items-start gap-3">
        <AlertTriangle className="w-4 h-4 text-amber-400 flex-shrink-0 mt-0.5" />
        <div className="space-y-1">
          <div className="text-xs font-bold text-amber-300">{drive.name}</div>
          <div className="text-[10px] font-mono text-amber-400/60">{drive.device} · {drive.size}</div>
        </div>
      </div>

      {/* Full warning copy */}
      <div className="px-4 py-3 text-xs text-amber-200/70 leading-relaxed border-b border-amber-500/10">
        This drive could not be fully identified. Selecting it may be safe, but the app cannot confirm
        it is a removable USB drive. You are responsible for verifying the device before proceeding.{' '}
        <span className="font-bold text-amber-300">ALL DATA WILL BE PERMANENTLY ERASED.</span>
      </div>

      {ackGranted ? (
        // Acknowledgement already granted — show the selectable card
        <div className="p-3">
          <DriveCard
            drive={drive}
            selected={selected}
            onSelect={onSelect}
            beginnerBlocked={false}
            advancedAckGranted={true}
            requireFullSize={requireFullSize}
          />
        </div>
      ) : (
        // Require typed acknowledgement
        <div className="px-4 py-3 space-y-3">
          <label className="text-[10px] text-white/35 block">
            Type exactly to unlock this drive:
            <span className="ml-1 font-mono text-amber-400/70 select-all">{SUSPICIOUS_ACK_TEXT}</span>
          </label>
          <input
            type="text"
            value={ackText}
            onChange={e => setAckText(e.target.value)}
            placeholder={SUSPICIOUS_ACK_TEXT}
            className="w-full px-3 py-2 bg-black/40 border border-white/10 rounded-xl text-xs font-mono text-white placeholder-white/15 focus:outline-none focus:border-amber-500/40 transition-colors"
          />
          <button
            disabled={!ackValid}
            onClick={onAckGranted}
            className={`w-full py-2.5 rounded-xl text-xs font-bold transition-all ${
              ackValid
                ? 'bg-amber-500/20 border border-amber-500/30 text-amber-300 hover:bg-amber-500/30 cursor-pointer'
                : 'bg-white/4 border border-white/8 text-white/20 cursor-not-allowed'
            }`}
          >
            Unlock drive for selection
          </button>
        </div>
      )}
    </div>
  );
}

// ── Selection review panel ────────────────────────────────────────────────────

interface SelectionReviewPanelProps {
  drive: DriveInfo;
  driveStillPresent: boolean;
  backupPolicy: EfiBackupPolicy | null;
  onRescan: () => void;
  onBack: () => void;
  onConfirm: () => void;
  requireFullSize: boolean;
}

function plainLanguageSummary(drive: DriveInfo, requireFullSize: boolean): { text: string; safe: boolean } {
  const { tier } = classifyDrive(drive, requireFullSize);
  if (tier === 'blocked') {
    return {
      text: 'This drive is blocked and cannot be selected. It is either your system disk or has been identified as an internal (non-removable) drive.',
      safe: false,
    };
  }
  if (tier === 'suspicious') {
    return {
      text: 'This drive could not be fully identified. Proceed only if you have verified — using another tool — that this is a removable USB drive with no important data.',
      safe: false,
    };
  }
  const parts: string[] = [];
  if (drive.removable === true) parts.push('reported as removable');
  if (drive.partitionTable === 'gpt') parts.push('GPT partition table');
  if (drive.isSystemDisk === false) parts.push('not your system disk');
  if (parts.length >= 2) {
    return {
      text: `This looks like a removable USB drive and appears safe to use (${parts.join(', ')}).`,
      safe: true,
    };
  }
  return {
    text: 'This drive appears to be a removable USB drive. Verify before continuing.',
    safe: true,
  };
}

function SelectionReviewPanel({ drive, driveStillPresent, backupPolicy, onRescan, onBack, onConfirm, requireFullSize }: SelectionReviewPanelProps) {
  const summary = plainLanguageSummary(drive, requireFullSize);
  const { tier, unreliable } = classifyDrive(drive, requireFullSize);

  const rows: { label: string; value: string; tooltip?: string }[] = [
    {
      label: 'Drive name',
      value: drive.name,
    },
    {
      label: 'Disk identifier',
      value: drive.device,
      tooltip: "The operating system's internal name for this drive. You will type this to confirm flashing.",
    },
    {
      label: 'Size',
      value: drive.size,
    },
    {
      label: 'Removable',
      value: drive.removable === true ? 'Yes (USB/External)' : drive.removable === false ? 'No (Internal)' : 'Unknown',
      tooltip: 'Whether the OS reports this drive as removable/external. Internal drives cannot be flashed.',
    },
    {
      label: 'Partition table',
      value: formatPartitionTable(drive.partitionTable),
      tooltip: 'GPT (GUID Partition Table) is required for OpenCore. MBR must be converted first.',
    },
    {
      label: 'System drive',
      value: drive.isSystemDisk === true ? 'YES — cannot flash' : drive.isSystemDisk === false ? 'No' : 'Unknown',
    },
  ];

  const canConfirm = tier !== 'blocked' && driveStillPresent && backupPolicy?.status !== 'blocked';

  return (
    <motion.div
      key="review-panel"
      initial={{ opacity: 0, x: 20 }}
      animate={{ opacity: 1, x: 0 }}
      exit={{ opacity: 0, x: -20 }}
      className="space-y-5"
    >
      {/* Back link + heading */}
      <div>
        <button
          onClick={onBack}
          className="flex items-center gap-1.5 text-xs text-white/30 hover:text-white/60 transition-colors cursor-pointer mb-4"
        >
          <ChevronDown className="w-3.5 h-3.5 rotate-90" /> Back to drive list
        </button>
        <h2 className="text-4xl font-bold text-white mb-1">Review Your Selection</h2>
        <p className="text-[#888888] font-medium text-sm">
          Verify the drive details below. You can still go back.
        </p>
      </div>

      {/* Plain-language summary banner */}
      <div className={`flex items-start gap-3 px-4 py-3.5 rounded-2xl border ${
        summary.safe
          ? 'bg-emerald-500/6 border-emerald-500/20'
          : 'bg-amber-500/6 border-amber-500/20'
      }`}>
        {summary.safe
          ? <CheckCircle2 className="w-4 h-4 text-emerald-400 flex-shrink-0 mt-0.5" />
          : <AlertTriangle className="w-4 h-4 text-amber-400 flex-shrink-0 mt-0.5" />}
        <p className={`text-xs leading-relaxed ${summary.safe ? 'text-emerald-200/80' : 'text-amber-200/80'}`}>
          {summary.text}
        </p>
      </div>

      {unreliable && (
        <div className="flex items-start gap-3 px-4 py-3 rounded-2xl bg-amber-500/10 border border-amber-500/20">
          <AlertTriangle className="w-4 h-4 text-amber-400 flex-shrink-0 mt-0.5" />
          <div className="space-y-1">
            <div className="text-[11px] font-bold text-amber-300 uppercase tracking-wide">Potential Reliability Risk</div>
            <p className="text-[11px] text-amber-200/60 leading-relaxed">
              This device uses a generic controller or reported unstable detection. 
              The operation may be slow or fail during verification.
            </p>
          </div>
        </div>
      )}

      {/* Drive details table */}
      <div className="rounded-2xl border border-white/8 bg-white/3 overflow-hidden">
        <div className="px-4 py-2.5 border-b border-white/5 flex items-center justify-between">
          <span className="text-[9px] font-bold uppercase tracking-widest text-white/25">Drive details</span>
          <button
            onClick={onRescan}
            className="flex items-center gap-1.5 text-[10px] text-white/30 hover:text-white/60 transition-colors cursor-pointer"
          >
            <RefreshCcw className="w-3 h-3" /> Re-scan now
          </button>
        </div>
        <div className="divide-y divide-white/4">
          {rows.map(({ label, value, tooltip }) => (
            <div key={label} className="flex items-center justify-between px-4 py-2.5">
              <span className="text-xs text-white/35 w-36 flex-shrink-0">
                {tooltip ? <JargonTooltip term={label} definition={tooltip} /> : label}
              </span>
              <span className="text-xs font-mono text-white/70 text-right min-w-0 truncate">{value}</span>
            </div>
          ))}
        </div>

        {/* Identifier instability notice */}
        <div className="px-4 py-3 border-t border-amber-500/15 bg-amber-500/4">
          <div className="flex items-start gap-2">
            <AlertTriangle className="w-3.5 h-3.5 text-amber-400 flex-shrink-0 mt-0.5" />
            <p className="text-[10px] text-amber-300/70 leading-relaxed">
              <span className="font-bold text-amber-300/90">Important:</span>{' '}
              If you disconnect and reconnect this drive, its identifier may change.
              Re-check the identifier above before confirming the flash.
            </p>
          </div>
        </div>

        {/* Drive no longer detected warning */}
        {!driveStillPresent && (
          <motion.div
            initial={{ opacity: 0, y: 4 }}
            animate={{ opacity: 1, y: 0 }}
            className="px-4 py-3 border-t border-red-500/20 bg-red-500/8"
          >
            <div className="flex items-start gap-2">
              <XCircle className="w-3.5 h-3.5 text-red-400 flex-shrink-0 mt-0.5" />
              <p className="text-xs text-red-300/80 leading-relaxed font-medium">
                The previously selected drive is no longer detected. Reconnect the drive or go back and select again.
              </p>
            </div>
          </motion.div>
        )}
      </div>

      <EfiBackupPanel policy={backupPolicy} />

      {/* Erasure warning */}
      <div className="flex gap-3 p-4 rounded-2xl bg-red-500/6 border border-red-500/15">
        <ShieldAlert className="w-4 h-4 text-red-400 flex-shrink-0 mt-0.5" />
        <div className="text-xs text-red-300/75 leading-relaxed">
          <span className="font-bold text-red-400">This drive will be completely erased and reformatted.</span>
          {' '}A final confirmation will appear before any data is written.
        </div>
      </div>

      {/* Footer actions */}
      <div className="flex flex-col gap-4 pt-2">
        <div className="flex items-center justify-between">
          <button
            onClick={onBack}
            className="text-sm text-white/30 hover:text-white/60 transition-colors cursor-pointer"
          >
            ← Choose a different drive
          </button>
          <motion.button
            onClick={onConfirm}
            disabled={!canConfirm}
            whileHover={canConfirm ? { scale: 1.02 } : undefined}
            whileTap={canConfirm ? { scale: 0.98 } : undefined}
            className={`px-8 py-3.5 rounded-xl text-sm font-bold transition-all flex items-center gap-2 ${
              canConfirm
                ? 'bg-red-600 text-white hover:bg-red-500 cursor-pointer shadow-lg shadow-red-600/20'
                : 'bg-white/5 text-white/25 cursor-not-allowed'
            }`}
          >
            <ShieldAlert className="w-4 h-4 opacity-70" />
            Confirm Flash to {drive.device} →
          </motion.button>
        </div>
        {!canConfirm && (
          <p className="text-[10px] text-red-400/60 text-center">
            {!driveStillPresent
              ? 'Drive disconnected — reconnect it or go back.'
              : backupPolicy?.status === 'blocked'
              ? 'Existing EFI could not be backed up safely on this target.'
              : 'This drive is blocked by safety rules.'}
          </p>
        )}
      </div>
    </motion.div>
  );
}

// ── Main component ────────────────────────────────────────────────────────────

interface Props {
  devices: DriveInfo[];
  selected: string | null;
  backupPolicy?: EfiBackupPolicy | null;
  onSelect: (dev: string) => void;
  onDeselect?: () => void;
  onRefresh: () => void;
  /** When provided, selecting a drive shows the review panel with this callback on confirm. */
  onConfirmDrive?: () => void;
  requireFullSize?: boolean;
}

export default function UsbStep({ devices, selected, backupPolicy = null, onSelect, onDeselect, onRefresh, onConfirmDrive, requireFullSize = true }: Props) {
  const [advancedOpen, setAdvancedOpen] = useState(false);
  // Per-device typed acknowledgements (device string → granted)
  const [ackGranted, setAckGranted] = useState<Set<string>>(new Set());
  // View: 'list' shows the drive list; 'review' shows the selection review panel
  const [view, setView] = useState<'list' | 'review'>('list');

  const safeDrives = devices.filter(d => classifyDrive(d, requireFullSize).tier === 'safe');
  const suspiciousDrives = devices.filter(d => classifyDrive(d, requireFullSize).tier === 'suspicious');
  const blockedDrives = devices.filter(d => classifyDrive(d, requireFullSize).tier === 'blocked');

  const selectedDrive = devices.find(d => d.device === selected);
  const selectedDriveStillPresent = !selected || devices.some(d => d.device === selected);

  // When the device list refreshes and the selected drive disappears, stay on review
  // but show the "no longer detected" warning (handled by driveStillPresent).

  const handleSelect = (dev: string) => {
    onSelect(dev);
    // If a confirm callback is wired, immediately transition to the review panel
    if (onConfirmDrive) {
      setView('review');
    }
  };

  const handleRescan = () => {
    onRefresh();
    // Stay on the current view; driveStillPresent will update reactively
  };

  const handleBack = () => {
    setView('list');
    // Clear the selection so the list shows all drives un-selected
    if (onDeselect) onDeselect();
  };

  const handleConfirm = () => {
    if (onConfirmDrive) {
      onConfirmDrive();
    }
  };

  // If viewing the review panel and we have a selected drive, render the review panel
  if (view === 'review' && selectedDrive && onConfirmDrive) {
    return (
      <SelectionReviewPanel
        drive={selectedDrive}
        driveStillPresent={selectedDriveStillPresent}
        backupPolicy={backupPolicy}
        onRescan={handleRescan}
        onBack={handleBack}
        onConfirm={handleConfirm}
        requireFullSize={requireFullSize}
      />
    );
  }

  return (
    <div className="space-y-6">
      <div>
        <h2 className="text-4xl font-bold text-white mb-2">Select a USB Drive</h2>
        <p className="text-[#888888] font-medium text-sm">
          Choose a removable USB drive (16 GB or larger). All data on it will be erased.
        </p>
      </div>

      {/* Erasure warning */}
      <div className="flex gap-3 p-4 rounded-2xl bg-red-500/6 border border-red-500/15">
        <ShieldAlert className="w-5 h-5 text-red-400 flex-shrink-0 mt-0.5" />
        <div className="text-xs text-red-300/75 leading-relaxed">
          <span className="font-bold text-red-400">The selected drive will be completely erased.</span>
          {' '}It will be reformatted as GPT/FAT32. Back up anything important first.
        </div>
      </div>

      {/* Drive list */}
      {devices.length === 0 ? (
        <div className="flex flex-col items-center justify-center py-14 space-y-4">
          <div className="w-20 h-20 rounded-full bg-white/4 flex items-center justify-center">
            <Usb className="w-10 h-10 text-[#333]" />
          </div>
          <p className="text-[#888888] font-medium">No USB drives detected</p>
          <p className="text-xs text-[#555555]">
            Plug in a removable USB drive of 16 GB or larger, then tap Refresh.
          </p>
          <button
            onClick={onRefresh}
            className="flex items-center gap-2 px-5 py-2.5 bg-white/5 border border-white/10 rounded-xl text-xs font-bold text-white hover:bg-white/10 transition-all cursor-pointer"
          >
            <RefreshCcw className="w-3.5 h-3.5" /> Refresh
          </button>
        </div>
      ) : (
        <div className="space-y-3">
          {/* Tier legend */}
          <div className="flex items-center gap-4 px-1 pb-1">
            <span className="text-[9px] font-bold uppercase tracking-widest text-white/20">Drive status</span>
            <span className="flex items-center gap-1 text-[10px] text-emerald-400/70">
              <CheckCircle2 className="w-2.5 h-2.5" /> Safe — recommended
            </span>
            {!BEGINNER_SAFETY_MODE && (
              <span className="flex items-center gap-1 text-[10px] text-amber-400/70">
                <AlertTriangle className="w-2.5 h-2.5" /> Suspicious — verify first
              </span>
            )}
            <span className="flex items-center gap-1 text-[10px] text-red-400/60">
              <XCircle className="w-2.5 h-2.5" /> Blocked
            </span>
          </div>

          {/* Safe drives */}
          {safeDrives.map((drive) => (
            <DriveCard
              key={drive.device}
              drive={drive}
              selected={selected === drive.device}
              onSelect={handleSelect}
              beginnerBlocked={false}
              advancedAckGranted={false}
              requireFullSize={requireFullSize}
            />
          ))}

          {/* Blocked drives — always shown, always non-selectable */}
          {blockedDrives.map((drive) => (
            <DriveCard
              key={drive.device}
              drive={drive}
              selected={false}
              onSelect={handleSelect}
              beginnerBlocked={true}
              advancedAckGranted={false}
              requireFullSize={requireFullSize}
            />
          ))}

          {/* In non-beginner mode: suspicious drives shown inline */}
          {!BEGINNER_SAFETY_MODE && suspiciousDrives.map((drive) => (
            <DriveCard
              key={drive.device}
              drive={drive}
              selected={selected === drive.device}
              onSelect={handleSelect}
              beginnerBlocked={false}
              advancedAckGranted={false}
              requireFullSize={requireFullSize}
            />
          ))}

          <button
            onClick={onRefresh}
            className="w-full flex items-center justify-center gap-2 py-3 text-xs text-[#555555] hover:text-[#888888] transition-colors cursor-pointer"
          >
            <RefreshCcw className="w-3 h-3" /> Refresh drive list
          </button>

          {/* Advanced devices disclosure — beginner mode only, when suspicious drives exist */}
          {BEGINNER_SAFETY_MODE && suspiciousDrives.length > 0 && (
            <div className="border-t border-white/5 pt-3">
              <button
                onClick={() => setAdvancedOpen(x => !x)}
                className="flex items-center gap-2 text-xs text-white/30 hover:text-white/55 transition-colors cursor-pointer px-1"
              >
                <Eye className="w-3.5 h-3.5" />
                Show advanced devices ({suspiciousDrives.length} hidden)
                <motion.span animate={{ rotate: advancedOpen ? 180 : 0 }}>
                  <ChevronDown className="w-3.5 h-3.5" />
                </motion.span>
              </button>

              <AnimatePresence>
                {advancedOpen && (
                  <motion.div
                    initial={{ height: 0, opacity: 0 }}
                    animate={{ height: 'auto', opacity: 1 }}
                    exit={{ height: 0, opacity: 0 }}
                    className="overflow-hidden"
                  >
                    <div className="mt-3 space-y-3">
                      {/* Section warning */}
                      <div className="flex items-start gap-2 px-3 py-2 rounded-xl bg-amber-500/6 border border-amber-500/15">
                        <AlertTriangle className="w-3.5 h-3.5 text-amber-400 flex-shrink-0 mt-0.5" />
                        <p className="text-[10px] text-amber-300/70 leading-relaxed">
                          These drives could not be fully identified. Each one requires typed acknowledgement before selection.
                          Only proceed if you are certain you are selecting the correct removable USB drive.
                        </p>
                      </div>

                      {suspiciousDrives.map((drive) => (
                        <AdvancedDriveRow
                          key={drive.device}
                          drive={drive}
                          selected={selected === drive.device}
                          onSelect={handleSelect}
                          ackGranted={ackGranted.has(drive.device)}
                          onAckGranted={() => setAckGranted(prev => new Set([...prev, drive.device]))}
                          requireFullSize={requireFullSize}
                        />
                      ))}
                    </div>
                  </motion.div>
                )}
              </AnimatePresence>
            </div>
          )}
        </div>
      )}

      {/* Device identity panel — shown in list view when a drive is selected and no confirm callback */}
      {!onConfirmDrive && (
        <AnimatePresence>
          {selectedDrive && (
            <DeviceIdentityPanel
              drive={selectedDrive}
              onRescan={handleRescan}
              driveStillPresent={selectedDriveStillPresent}
            />
          )}
        </AnimatePresence>
      )}
    </div>
  );
}
