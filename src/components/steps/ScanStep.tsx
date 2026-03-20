import { motion, AnimatePresence } from 'motion/react';
import { Cpu, Monitor, Server, MemoryStick, AlertTriangle, Info } from 'lucide-react';
import BrandIcon from '../BrandIcon';
import type { HardwareProfile } from '../../../electron/configGenerator';

interface Props {
  progress: number;
  profile: HardwareProfile | null;
}

const specs = (profile: HardwareProfile | null) => [
  { icon: Cpu,    label: 'Processor',   value: profile?.cpu ?? null,         color: 'blue'   },
  { icon: Monitor,label: 'Graphics',    value: profile?.gpu ?? null,         color: 'purple' },
  { icon: Server, label: profile?.isVM ? 'Platform' : 'Motherboard', value: profile?.isVM ? 'Virtual Machine' : (profile?.motherboard ?? null), color: 'emerald'},
  { icon: MemoryStick,label:'Memory',   value: profile?.ram ?? null,         color: 'amber'  },
];

const colorMap: Record<string, string> = {
  blue:    'bg-blue-500/10 text-blue-400 border-blue-500/15',
  purple:  'bg-purple-500/10 text-purple-400 border-purple-500/15',
  emerald: 'bg-emerald-500/10 text-emerald-400 border-emerald-500/15',
  amber:   'bg-amber-500/10 text-amber-400 border-amber-500/15',
};

export default function ScanStep({ progress, profile }: Props) {
  const items = specs(profile);
  const discovered = Math.floor((progress / 100) * items.length);

  // Confidence indicator — only shown once scan is complete (progress === 100 or profile present)
  const confidence = profile?.scanConfidence;
  const scanComplete = progress >= 100 || profile !== null;

  return (
    <div className="flex flex-col items-center justify-center h-full py-6 space-y-12">
      {/* Tactical Radar */}
      <div className="relative w-32 h-32 flex items-center justify-center">
        <div className="radar-ring !border-blue-500/30" />
        <div className="radar-ring !border-blue-400/20 !animation-delay-[0.7s] !scale-150" />
        <div className="radar-ring !border-white/10 !animation-delay-[1.5s]" />

        <motion.div
          initial={{ opacity: 0, y: 20 }}
          animate={{
            opacity: 1,
            y: [0, -10, 0],
            transition: {
              opacity: { duration: 1 },
              y: { duration: 6, repeat: Infinity, ease: "easeInOut" }
            }
          }}
          className="relative z-10 p-5 rounded-3xl bg-white/5 border border-white/10 backdrop-blur-xl shadow-2xl"
        >
          <BrandIcon className="w-10 h-10 text-white" />
          <motion.div
            animate={{ opacity: [0.1, 0.4, 0.1] }}
            transition={{ duration: 1.5, repeat: Infinity }}
            className="absolute inset-0 bg-blue-500/20 rounded-3xl blur-xl"
          />
        </motion.div>
      </div>

      <div className="text-center space-y-3">
        <motion.h2
          initial={{ opacity: 0, y: 10 }}
          animate={{ opacity: 1, y: 0 }}
          className="text-4xl font-black text-white tracking-tight"
        >
          Scanning Hardware
        </motion.h2>
        <p className="text-[#888888] text-sm font-medium tracking-wide">
          {progress < 40 ? 'Identifying processor architecture and generation…'
            : progress < 70 ? 'Classifying GPU and checking macOS driver support…'
            : progress < 90 ? 'Reading board identity and memory configuration…'
            : 'Mapping hardware to OpenCore configuration…'}
        </p>
      </div>

      {/* Discovered hardware cards */}
      <div className="grid grid-cols-2 gap-4 w-full max-w-md">
        {items.map((item, i) => {
          const Icon = item.icon;
          const visible = i < discovered || profile !== null;
          return (
            <motion.div
              key={item.label}
              initial={{ opacity: 0, scale: 0.8, y: 20 }}
              animate={visible ? { opacity: 1, scale: 1, y: 0 } : {}}
              transition={{
                type: 'spring',
                stiffness: 200,
                damping: 20,
                delay: i * 0.1
              }}
              className={`group relative p-5 rounded-2xl border ${colorMap[item.color]} bg-white/3 backdrop-blur-md overflow-hidden`}
            >
              <div className="absolute inset-0 bg-gradient-to-br from-white/5 to-transparent opacity-0 group-hover:opacity-100 transition-opacity" />
              <Icon className="w-5 h-5 mb-3 relative z-10" />
              <div className="text-[9px] font-bold uppercase tracking-[0.2em] opacity-40 mb-1">{item.label}</div>
              <div className="text-sm font-bold text-white truncate relative z-10">
                {item.value ?? <span className="opacity-20">—</span>}
              </div>
              {visible && item.value && (
                <motion.div
                  initial={{ x: '-100%' }}
                  animate={{ x: '100%' }}
                  transition={{ duration: 1, repeat: Infinity, repeatDelay: 3 }}
                  className="absolute top-0 bottom-0 w-8 bg-white/5 skew-x-[-20deg]"
                />
              )}
            </motion.div>
          );
        })}
      </div>

      {/* Confidence indicator — shown after scan completes */}
      <AnimatePresence>
        {scanComplete && confidence && confidence !== 'high' && (
          <motion.div
            key="confidence-banner"
            initial={{ opacity: 0, y: 8 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0 }}
            className="w-full max-w-md"
          >
            {confidence === 'low' ? (
              <div className="flex items-start gap-3 px-4 py-3 rounded-2xl bg-amber-500/8 border border-amber-500/20">
                <AlertTriangle className="w-4 h-4 text-amber-400 flex-shrink-0 mt-0.5" />
                <div className="text-xs text-amber-300/80 leading-relaxed">
                  <span className="font-bold text-amber-300">Hardware detection confidence is low</span>{' '}
                  — config generation may be inaccurate. Manual verification of the generated config.plist
                  is recommended before booting.
                </div>
              </div>
            ) : (
              <div className="flex items-start gap-3 px-4 py-3 rounded-2xl bg-amber-500/4 border border-amber-500/10">
                <Info className="w-3.5 h-3.5 text-amber-400/60 flex-shrink-0 mt-0.5" />
                <div className="text-[10px] text-amber-300/55 leading-relaxed">
                  Some hardware details were inferred, not directly confirmed.
                </div>
              </div>
            )}
          </motion.div>
        )}
      </AnimatePresence>

      {/* Progress tracking */}
      <div className="w-full max-w-md flex flex-col items-center space-y-4">
        <div className="w-full h-1 bg-white/5 rounded-full overflow-hidden relative">
          <motion.div
            className="h-full bg-gradient-to-r from-blue-600 via-blue-400 to-blue-500 rounded-full"
            animate={{ width: `${progress}%` }}
            transition={{ type: 'spring', stiffness: 50, damping: 20 }}
          />
        </div>
        <span className="text-[10px] font-mono text-white/20 tracking-widest">{Math.floor(progress)}%</span>
      </div>
    </div>
  );
}
