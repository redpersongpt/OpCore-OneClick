import React, { useState } from 'react';
import { motion } from 'motion/react';
import { Check, AlertTriangle, HelpCircle, Usb, Settings, Download, Info } from 'lucide-react';
import BrandIcon from '../BrandIcon';

interface Props {
  onContinue: () => void;
}

export default function WelcomeStep({ onContinue }: Props) {
  const [acknowledged, setAcknowledged] = useState(false);

  return (
    <div className="flex flex-col h-full space-y-6 overflow-y-auto custom-scrollbar pr-1">
      {/* Header */}
      <div className="flex-shrink-0 flex items-center gap-4">
        <div className="w-12 h-12 rounded-2xl bg-white/5 border border-white/10 flex items-center justify-center flex-shrink-0">
          <BrandIcon className="w-7 h-7 text-white" />
        </div>
        <div>
          <h2 className="text-4xl font-bold text-white tracking-tight">Before You Start</h2>
          <p className="text-[#888888] text-sm font-medium mt-1">
            What this app does, what it doesn't, and what you need to know.
          </p>
        </div>
      </div>

      {/* What this app does */}
      <div className="flex-shrink-0 rounded-2xl border border-white/8 bg-white/3 overflow-hidden">
        <div className="px-5 py-3 border-b border-white/5">
          <span className="text-[9px] font-bold uppercase tracking-widest text-emerald-400/60">What this app does</span>
        </div>
        <div className="divide-y divide-white/4">
          {[
            { icon: Usb,      text: 'Prepares a bootable USB drive with the macOS recovery installer' },
            { icon: Settings, text: 'Generates an OpenCore bootloader configuration for your hardware' },
            { icon: HelpCircle, text: 'Guides you through the BIOS settings your target PC needs' },
          ].map(({ icon: Icon, text }) => (
            <div key={text} className="flex items-start gap-3 px-5 py-3">
              <Icon className="w-4 h-4 text-emerald-400 flex-shrink-0 mt-0.5" />
              <span className="text-sm text-white/60 leading-relaxed">{text}</span>
            </div>
          ))}
        </div>
      </div>

      {/* What this app does NOT do */}
      <div className="flex-shrink-0 rounded-2xl border border-white/8 bg-white/3 overflow-hidden">
        <div className="px-5 py-3 border-b border-white/5">
          <span className="text-[9px] font-bold uppercase tracking-widest text-amber-400/60">What this app does NOT do</span>
        </div>
        <div className="divide-y divide-white/4">
          {[
            {
              icon: Download,
              text: 'Install macOS — you boot from the USB and run the installer yourself',
            },
            {
              icon: AlertTriangle,
              text: 'Guarantee compatibility — always verify against the Dortania guide for your hardware',
            },
            {
              icon: Settings,
              text: 'Configure BIOS — you apply those settings yourself on the target PC',
            },
            {
              icon: AlertTriangle,
              text: 'Preserve USB data — the selected drive will be completely erased',
            },
          ].map(({ icon: Icon, text }) => (
            <div key={text} className="flex items-start gap-3 px-5 py-3">
              <Icon className="w-4 h-4 text-amber-400/70 flex-shrink-0 mt-0.5" />
              <span className="text-sm text-white/50 leading-relaxed">{text}</span>
            </div>
          ))}
        </div>
      </div>

      {/* Beta warning */}
      <div className="flex flex-col gap-3">
        <div className="flex items-start gap-3 px-4 py-4 rounded-2xl bg-amber-500/8 border border-amber-500/25">
          <AlertTriangle className="w-5 h-5 text-amber-400 flex-shrink-0 mt-0.5" />
          <div className="space-y-1.5">
            <div className="text-sm font-bold text-amber-300">Beta — intermediate users only</div>
            <div className="text-xs text-amber-200/70 leading-relaxed space-y-1">
              <p>You should be comfortable with BIOS settings and disk formatting.</p>
              <p>Only use USB drives that contain no important data.</p>
              <p>Community project — not affiliated with or supported by Apple.</p>
            </div>
          </div>
        </div>

        {/* Linux specific note */}
        {navigator.userAgent.includes('Linux') && (
          <div className="flex items-start gap-3 px-4 py-3 rounded-2xl bg-blue-500/5 border border-blue-500/15">
            <Info className="w-4 h-4 text-blue-400 flex-shrink-0 mt-0.5" />
            <div className="space-y-1">
              <div className="text-[11px] font-bold text-blue-300 uppercase tracking-wide">Linux Installation Note</div>
              <p className="text-[10px] text-blue-200/60 leading-relaxed">
                For the best experience, use the <span className="font-bold text-blue-200/80">.deb</span> package. 
                If using the <span className="font-bold text-blue-200/80">AppImage</span>, you must manually mark it as executable (<span className="font-mono bg-white/5 px-1 rounded">chmod +x</span>) after download.
              </p>
            </div>
          </div>
        )}
      </div>

      {/* Acknowledgement checkbox */}
      <div className="flex-shrink-0">
        <label className="flex items-start gap-3 cursor-pointer group">
          <div
            onClick={() => setAcknowledged(x => !x)}
            className={`w-5 h-5 rounded-md border flex-shrink-0 mt-0.5 flex items-center justify-center transition-all cursor-pointer ${
              acknowledged
                ? 'bg-blue-500 border-blue-500'
                : 'bg-white/5 border-white/15 group-hover:border-white/30'
            }`}
          >
            {acknowledged && <Check className="w-3 h-3 text-white stroke-[3px]" />}
          </div>
          <span
            onClick={() => setAcknowledged(x => !x)}
            className={`text-sm leading-relaxed transition-colors ${acknowledged ? 'text-white/70' : 'text-white/40'}`}
          >
            I understand this is beta software, I will use a non-critical USB drive, and I accept responsibility for BIOS configuration.
          </span>
        </label>
      </div>

      {/* Footer */}
      <div className="flex-shrink-0 pt-2 border-t border-white/5 flex items-center justify-end">
        <motion.button
          onClick={onContinue}
          disabled={!acknowledged}
          whileHover={acknowledged ? { scale: 1.02 } : undefined}
          whileTap={acknowledged ? { scale: 0.98 } : undefined}
          className={`px-8 py-3 rounded-xl text-sm font-bold transition-all ${
            acknowledged
              ? 'bg-blue-600 text-white hover:bg-blue-500 cursor-pointer shadow-lg shadow-blue-600/20'
              : 'bg-white/5 text-white/30 cursor-not-allowed'
          }`}
        >
          {acknowledged ? 'Continue →' : 'Accept the terms above to continue'}
        </motion.button>
      </div>
    </div>
  );
}
