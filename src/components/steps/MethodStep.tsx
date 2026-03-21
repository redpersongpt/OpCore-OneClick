import React from 'react';
import { motion } from 'motion/react';
import { Usb, HardDrive, ChevronRight, Info } from 'lucide-react';

interface Props {
  onSelect: (method: 'usb' | 'partition') => void;
  onBack: () => void;
  platform?: string;
}

export default function MethodStep({ onSelect }: Props) {
  return (
    <div className="h-full flex flex-col space-y-8 py-4">
      <div className="text-left animate-in fade-in slide-in-from-left duration-700">
        <h2 className="text-4xl font-bold text-white tracking-tight mb-2">Deployment Method</h2>
        <p className="text-[#888888] text-sm font-medium mt-2">
          Choose where to place the installer.
        </p>
      </div>

      <div className="grid grid-cols-2 gap-6">
        {/* USB Method */}
        <motion.button
          whileHover={{ scale: 1.02, backgroundColor: 'rgba(255,255,255,0.05)' }}
          whileTap={{ scale: 0.98 }}
          onClick={() => onSelect('usb')}
          className="flex flex-col items-start p-8 rounded-3xl bg-white/3 border border-white/6 text-left transition-all group relative overflow-hidden"
        >
          <div className="w-14 h-14 rounded-2xl bg-blue-500/10 flex items-center justify-center mb-6 group-hover:bg-blue-500/20 transition-colors">
            <Usb className="w-7 h-7 text-blue-400" />
          </div>
          <h3 className="text-xl font-bold text-white mb-2">USB Flash Drive</h3>
          <p className="text-sm text-[#777] leading-relaxed mb-6">
            Write the installer to a USB drive. Best for most installs.
          </p>
          <div className="mt-auto flex items-center text-blue-400 font-bold text-sm">
            Recommended <ChevronRight className="w-4 h-4 ml-1" />
          </div>
          <div className="absolute top-0 right-0 p-4 opacity-5 group-hover:opacity-10 transition-opacity">
            <Usb className="w-24 h-24 rotate-12" />
          </div>
        </motion.button>

        {/* Partition Method */}
        <motion.button
          whileHover={{ scale: 1.02, backgroundColor: 'rgba(255,255,255,0.05)' }}
          whileTap={{ scale: 0.98 }}
          onClick={() => onSelect('partition')}
          className="flex flex-col items-start p-8 rounded-3xl bg-white/3 border border-white/6 text-left transition-all group relative overflow-hidden"
        >
          <div className="w-14 h-14 rounded-2xl bg-purple-500/10 flex items-center justify-center mb-6 group-hover:bg-purple-500/20 transition-colors">
            <HardDrive className="w-7 h-7 text-purple-400" />
          </div>
          <h3 className="text-xl font-bold text-white mb-2">Local Partition</h3>
          <p className="text-sm text-[#777] leading-relaxed mb-6">
            Create a small boot partition on this PC. No USB needed, but it changes your disk layout.
          </p>
          <div className="mt-auto flex items-center text-purple-400 font-bold text-sm">
            Advanced only <ChevronRight className="w-4 h-4 ml-1" />
          </div>
          <div className="absolute top-0 right-0 p-4 opacity-5 group-hover:opacity-10 transition-opacity">
            <HardDrive className="w-24 h-24 -rotate-12" />
          </div>
        </motion.button>
      </div>

      <div className="p-5 rounded-2xl bg-amber-500/5 border border-amber-500/10 flex gap-4">
        <Info className="w-5 h-5 text-amber-500 flex-shrink-0 mt-0.5" />
        <div className="space-y-1">
          <p className="text-xs font-bold text-amber-200/90 uppercase tracking-widest">About local partitioning</p>
          <p className="text-xs text-amber-200/50 leading-relaxed">
            This changes your disk layout. Back up anything important first.
          </p>
        </div>
      </div>
    </div>
  );
}
