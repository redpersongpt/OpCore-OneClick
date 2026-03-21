import React, { useState } from 'react';
import { motion } from 'motion/react';
import { Check, Clock, Usb, WifiOff, HardDrive, Cpu, Shield } from 'lucide-react';

interface Props {
  onContinue: () => void;
}

interface Requirement {
  id: string;
  icon: React.ElementType;
  title: string;
  description: string;
  color: string;
  source: string; // Dortania guideline ref
}

const REQUIREMENTS: Requirement[] = [
  {
    id: 'time',
    icon: Clock,
    title: 'Time & patience',
    description: 'Do not start if you have deadlines. Hackintoshes are not work machines. Budget 2–4 hours for a first build.',
    color: 'amber',
    source: 'Dortania prerequisites #1',
  },
  {
    id: 'bios',
    icon: Shield,
    title: 'BIOS updated to latest',
    description: 'Current firmware usually gives the best OpenCore starting point. Verify any board-specific caveats in the Dortania guide before updating.',
    color: 'blue',
    source: 'Dortania prerequisites #6',
  },
  {
    id: 'usb',
    icon: Usb,
    title: 'USB drive ≥ 16 GB',
    description: 'Minimum 16 GB if creating the USB on macOS. At least 4 GB if using Windows or Linux. All data will be erased.',
    color: 'purple',
    source: 'Dortania prerequisites #4',
  },
  {
    id: 'ethernet',
    icon: WifiOff,
    title: 'Ethernet connection (not WiFi)',
    description: 'A physical Ethernet port or a compatible USB-Ethernet adapter is required. Most built-in WiFi cards are unsupported by macOS. Android USB tethering (HoRNDIS) is an alternative.',
    color: 'emerald',
    source: 'Dortania prerequisites #5',
  },
  {
    id: 'disk',
    icon: HardDrive,
    title: '15 GB+ free disk space',
    description: '15 GB minimum on Windows/Linux (on your OS drive). 30 GB minimum on macOS. The recovery image and EFI take significant space.',
    color: 'rose',
    source: 'Dortania prerequisites #5',
  },
  {
    id: 'hardware',
    icon: Cpu,
    title: 'Know your hardware',
    description: 'You need: CPU name & generation, GPU model, storage type (SATA/NVMe/AHCI), Ethernet chipset, and WiFi/BT card. We will auto-detect these in the next step.',
    color: 'sky',
    source: 'Dortania prerequisites #3',
  },
];

const colorMap: Record<string, { bg: string; border: string; icon: string; check: string }> = {
  amber:   { bg: 'bg-amber-500/5',   border: 'border-amber-500/15',   icon: 'text-amber-400',   check: 'bg-amber-500' },
  blue:    { bg: 'bg-blue-500/5',    border: 'border-blue-500/15',    icon: 'text-blue-400',    check: 'bg-blue-500' },
  purple:  { bg: 'bg-purple-500/5',  border: 'border-purple-500/15',  icon: 'text-purple-400',  check: 'bg-purple-500' },
  emerald: { bg: 'bg-emerald-500/5', border: 'border-emerald-500/15', icon: 'text-emerald-400', check: 'bg-emerald-500' },
  rose:    { bg: 'bg-rose-500/5',    border: 'border-rose-500/15',    icon: 'text-rose-400',    check: 'bg-rose-500' },
  sky:     { bg: 'bg-sky-500/5',     border: 'border-sky-500/15',     icon: 'text-sky-400',     check: 'bg-sky-500' },
  indigo:  { bg: 'bg-indigo-500/5',  border: 'border-indigo-500/15',  icon: 'text-indigo-400',  check: 'bg-indigo-500' },
};

export default function PrereqStep({ onContinue }: Props) {
  const [checked, setChecked] = useState<Set<string>>(new Set());
  const toggle = (id: string) => setChecked(p => { const n = new Set(p); n.has(id) ? n.delete(id) : n.add(id); return n; });
  const allDone = REQUIREMENTS.every(r => checked.has(r.id));

  return (
    <div className="flex flex-col h-full space-y-5">
      <div className="flex-shrink-0">
        <h2 className="text-4xl font-bold text-white mb-2">Before You Begin</h2>
        <p className="text-[#888888] text-sm font-medium">
          From Dortania's OpenCore Install Guide — confirm each item before continuing.
        </p>
      </div>

      {/* Beta disclaimer banner */}
      <div className="flex-shrink-0 flex items-start gap-3 px-4 py-3 rounded-2xl bg-amber-500/10 border border-amber-500/25">
        <span className="text-amber-400 text-base leading-none mt-0.5">⚠</span>
        <div className="text-xs leading-relaxed text-amber-200/80">
          <span className="font-bold text-amber-300">Beta software — intermediate users only.</span>{' '}
          This tool automates OpenCore setup, but{' '}
          <span className="font-semibold text-amber-200">BIOS settings must be verified manually</span>{' '}
          on your target machine. Only use USB drives with no important data.
        </div>
      </div>

      <div className="flex-1 overflow-y-auto custom-scrollbar space-y-3 pr-1">
        {REQUIREMENTS.map((req, i) => {
          const Icon = req.icon;
          const c = colorMap[req.color];
          const done = checked.has(req.id);
          return (
            <motion.div
              layout
              key={req.id}
              initial={{ opacity: 0, x: -20 }}
              animate={{ opacity: 1, x: 0 }}
              transition={{ 
                type: 'spring',
                stiffness: 260,
                damping: 20,
                delay: i * 0.04 
              }}
              onClick={() => toggle(req.id)}
              className={`magnetic-glow group flex items-start gap-4 p-5 rounded-3xl border cursor-pointer transition-all ${
                done ? `${c.bg} ${c.border}` : 'bg-white/3 border-white/6 hover:bg-white/6 hover:border-white/12'
              }`}
            >
              {/* Icon */}
              <div className={`w-10 h-10 rounded-xl flex items-center justify-center flex-shrink-0 mt-0.5 transition-all ${
                done ? c.bg + ' ' + c.border + ' border' : 'bg-white/6'
              }`}>
                <Icon className={`w-5 h-5 ${done ? c.icon : 'text-[#555]'}`} />
              </div>

              {/* Content */}
              <div className="flex-1 min-w-0">
                <div className={`text-sm font-bold transition-colors ${done ? 'text-white' : 'text-[#AAAAAA]'}`}>
                  {req.title}
                </div>
                <div className="text-xs text-[#666666] leading-relaxed mt-0.5">{req.description}</div>
              </div>

              <motion.div 
                animate={{ 
                  scale: done ? 1.1 : 1,
                  backgroundColor: done ? colorMap[req.color].check.replace('bg-', '') : 'rgba(255,255,255,0.08)'
                }}
                className={`w-7 h-7 rounded-full flex items-center justify-center flex-shrink-0 mt-0.5 transition-all ${
                  done ? c.check : 'bg-white/8 group-hover:bg-white/12'
                }`}
              >
                {done && (
                  <motion.div initial={{ scale: 0 }} animate={{ scale: 1 }} transition={{ type: 'spring' }}>
                    <Check className="w-4 h-4 text-white stroke-[3px]" />
                  </motion.div>
                )}
              </motion.div>
            </motion.div>
          );
        })}

        <div className="mt-4 pt-4 border-t border-white/10 px-1">
          <div className="text-xs text-white/45 leading-relaxed">
            You will choose USB flash or local partition later in the setup flow.
          </div>
        </div>
      </div>

      {/* Footer */}
      <div className="flex-shrink-0 pt-4 border-t border-white/5 flex items-center justify-between">
        <div className="text-xs text-[#444444]">
          {checked.size} / {REQUIREMENTS.length} confirmed
        </div>
        <button
          onClick={onContinue}
          disabled={!allDone}
          className={`px-8 py-3 rounded-xl text-sm font-bold transition-all ${
            allDone
              ? 'bg-blue-600 text-white hover:bg-blue-500 cursor-pointer shadow-lg shadow-blue-600/20'
              : 'bg-white/5 text-white/30 cursor-not-allowed'
          }`}
        >
          {allDone ? 'Continue →' : `${REQUIREMENTS.length - checked.size} items remaining`}
        </button>
      </div>
    </div>
  );
}
