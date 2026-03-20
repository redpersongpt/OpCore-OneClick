import { motion } from 'motion/react';
import {
  Check, FolderOpen, Lock, ShieldCheck, RotateCcw,
  AlertTriangle, ExternalLink, Usb, Monitor, HardDrive, Settings, LogIn,
} from 'lucide-react';
import type { HardwareProfile } from '../../../electron/configGenerator';
import { getRequiredResources } from '../../../electron/configGenerator';
import CopyDiagnosticsButton from '../CopyDiagnosticsButton';

interface Props {
  profile: HardwareProfile;
  efiPath: string | null;
  productionLocked: boolean;
  onOpenFolder: () => void;
  onProductionLock: () => void;
  onRestart: () => void;
  onTroubleshoot: () => void;
}

const WHAT_COMES_NEXT = [
  {
    icon: Usb,
    title: 'Eject the USB drive safely',
    detail: 'Before removing the drive, eject it using your operating system to avoid corruption.',
  },
  {
    icon: HardDrive,
    title: 'Move to your target PC',
    detail: 'Take the USB drive to the machine you want to install macOS on.',
  },
  {
    icon: Settings,
    title: 'Review the BIOS settings',
    detail: 'Confirm the required BIOS settings are applied on the target machine before booting.',
  },
  {
    icon: Monitor,
    title: 'Boot from the USB drive',
    detail: 'On the target PC, open the boot menu (usually F12, Del, or Option) and select the USB drive.',
  },
  {
    icon: LogIn,
    title: 'Run the macOS installer',
    detail: 'OpenCore will appear first. Choose "Install macOS", format the target drive as APFS in Disk Utility, then follow the installer. It reboots several times — this is normal.',
  },
];

const WHAT_WAS_NOT_DONE = [
  'macOS was not installed — you run the installer yourself after booting from USB.',
  'BIOS settings were verified but not written to firmware — confirm them on the target machine before booting.',
  'Full compatibility was not guaranteed — check the Dortania guide for edge cases.',
];

export default function CompleteStep({ profile, efiPath, productionLocked, onOpenFolder, onProductionLock, onRestart, onTroubleshoot }: Props) {
  const { kexts, ssdts } = getRequiredResources(profile);

  return (
    <div className="flex flex-col space-y-6 py-4">
      {/* Success mark + heading */}
      <div className="flex items-center gap-5">
        <motion.div
          initial={{ scale: 0.5, opacity: 0 }}
          animate={{ scale: 1, opacity: 1 }}
          transition={{ type: 'spring', stiffness: 260, damping: 20 }}
          className="w-14 h-14 rounded-2xl bg-emerald-500 flex items-center justify-center shadow-[0_0_40px_rgba(34,197,94,0.25)] flex-shrink-0"
        >
          <Check className="w-7 h-7 text-black stroke-[3px]" />
        </motion.div>
        <div>
          <h2 className="text-4xl font-bold text-white tracking-tight">Setup Complete</h2>
          <p className="text-[#888888] text-sm font-medium mt-1">
            Your EFI and installer media are ready. Boot from the USB drive on the target machine and run the macOS installer.
          </p>
        </div>
      </div>

      {/* Summary cards */}
      <div className="grid grid-cols-2 gap-3">
        <InfoCard label="Target" value={profile.targetOS} />
        <InfoCard label="Identity" value={profile.smbios} />
        <InfoCard label="Extensions" value={`${kexts.length} kexts`} />
        <InfoCard label="ACPI" value={`${ssdts.length} SSDTs`} />
      </div>

      {/* What comes next — numbered checklist */}
      <div className="rounded-3xl border border-emerald-500/20 bg-emerald-500/[0.03] overflow-hidden shadow-xl shadow-emerald-500/[0.05]">
        <div className="px-6 py-4 border-b border-emerald-500/10 flex items-center justify-between">
          <span className="text-xs font-bold uppercase tracking-widest text-emerald-400">Action Checklist</span>
          <span className="text-[10px] font-medium text-emerald-400/50">Follow in order</span>
        </div>
        <div className="divide-y divide-emerald-500/10">
          {WHAT_COMES_NEXT.map(({ icon: Icon, title, detail }, i) => (
            <div key={title} className="flex items-start gap-5 px-6 py-5 hover:bg-emerald-500/[0.02] transition-colors">
              <div className="flex-shrink-0 flex items-center justify-center w-8 h-8 rounded-xl bg-emerald-500/10 text-xs font-bold text-emerald-400 border border-emerald-500/20">
                {i + 1}
              </div>
              <div className="space-y-1">
                <div className="text-sm font-bold text-white flex items-center gap-2">
                  <Icon className="w-4 h-4 text-emerald-400/60" />
                  {title}
                </div>
                <div className="text-xs text-white/40 leading-relaxed">{detail}</div>
              </div>
            </div>
          ))}
        </div>
      </div>

      {/* What was NOT done automatically */}
      <div className="rounded-2xl border border-amber-500/20 bg-amber-500/4 overflow-hidden">
        <div className="px-5 py-3 border-b border-amber-500/10 flex items-center gap-2">
          <AlertTriangle className="w-3.5 h-3.5 text-amber-400/70" />
          <span className="text-[9px] font-bold uppercase tracking-widest text-amber-400/60">Not included in this setup</span>
        </div>
        <div className="divide-y divide-amber-500/8">
          {WHAT_WAS_NOT_DONE.map((item) => (
            <div key={item} className="flex items-start gap-3 px-5 py-3">
              <AlertTriangle className="w-3.5 h-3.5 text-amber-400/50 flex-shrink-0 mt-0.5" />
              <span className="text-xs text-amber-200/60 leading-relaxed">{item}</span>
            </div>
          ))}
        </div>
      </div>

      {/* Boot-args */}
      <div className="p-4 rounded-2xl bg-black/40 border border-white/6 space-y-1.5">
        <div className="text-[10px] text-[#555555] font-bold uppercase tracking-widest">Boot arguments</div>
        <div className="font-mono text-xs text-blue-300 break-all">{profile.bootArgs}</div>
      </div>

      {/* Primary action */}
      <button
        onClick={onRestart}
        className="w-full py-4 bg-blue-600 text-white rounded-2xl font-bold text-base hover:bg-blue-500 transition-all shadow-xl shadow-blue-600/20 cursor-pointer flex flex-col items-center justify-center gap-0.5"
      >
        <div className="flex items-center gap-2">
          <RotateCcw className="w-5 h-5" /> Restart to Boot Menu
        </div>
        <div className="text-[10px] text-white/50 font-medium">Save all work first — your PC will restart</div>
      </button>

      {/* Secondary actions */}
      <div className="flex gap-3">
        <button
          onClick={onOpenFolder}
          className="flex-1 py-3 bg-white/5 border border-white/10 text-white rounded-2xl font-bold text-sm hover:bg-white/10 transition-all cursor-pointer flex items-center justify-center gap-2"
        >
          <FolderOpen className="w-4 h-4" /> View EFI
        </button>

        {!productionLocked ? (
          <button
            onClick={onProductionLock}
            className="flex-1 py-3 bg-amber-600/15 border border-amber-500/20 text-amber-400 rounded-2xl font-bold text-sm hover:bg-amber-600/25 transition-all cursor-pointer flex items-center justify-center gap-2"
          >
            <Lock className="w-4 h-4" /> Production Lock
          </button>
        ) : (
          <div className="flex-1 py-3 bg-emerald-500/10 border border-emerald-500/20 text-emerald-400 rounded-2xl font-bold text-sm flex items-center justify-center gap-2">
            <ShieldCheck className="w-4 h-4" /> Secured
          </div>
        )}

        <button
          onClick={onTroubleshoot}
          className="flex-1 py-3 bg-white/5 border border-white/10 text-white rounded-2xl font-bold text-sm hover:bg-white/10 transition-all cursor-pointer flex items-center justify-center gap-2"
        >
          <AlertTriangle className="w-4 h-4" /> Troubleshoot
        </button>
      </div>

      {/* Diagnostics + issue report */}
      <div className="flex items-center gap-3 pt-2 border-t border-white/5">
        <CopyDiagnosticsButton />
        <a
          href="https://github.com/redpersongpt/macOS-One-Click/issues/new"
          target="_blank"
          rel="noopener noreferrer"
          className="flex items-center gap-1.5 text-xs text-white/25 hover:text-white/50 transition-colors cursor-pointer"
        >
          <ExternalLink className="w-3 h-3" /> Report an issue
        </a>
      </div>
    </div>
  );
}

function InfoCard({ label, value }: { label: string; value: string }) {
  return (
    <div className="p-4 rounded-2xl bg-white/3 border border-white/6">
      <div className="text-[10px] text-[#555555] font-bold uppercase tracking-widest">{label}</div>
      <div className="text-sm font-bold text-white mt-1 truncate">{value}</div>
    </div>
  );
}
