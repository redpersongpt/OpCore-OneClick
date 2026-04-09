import { useEffect, useState, useCallback, useRef, useMemo } from 'react';
import { AnimatePresence, motion } from 'motion/react';
import { useWizard } from '../stores/wizard';
import { useHardware } from '../stores/hardware';
import { useEfi } from '../stores/efi';
import { useCompatibility } from '../stores/compatibility';
import { buildProfile } from '../lib/buildProfile';
import { formatMacOsLabel } from '../lib/macosVersion';
import { makeDemoBuildResult } from '../lib/demoData';
import { detectPlatform, getQuotePool, getRandomQuote } from '../lib/buildQuotes';
import { onTaskUpdate } from '../bridge/events';
import { EmptyState } from '../components/feedback/EmptyState';
import { Progress } from '../components/ui/Progress';
import { Badge } from '../components/ui/Badge';
import { WarningBanner } from '../components/ui/WarningBanner';
import { Button } from '../components/ui/Button';
import { BuildPhaseRow } from '../components/build/BuildPhaseRow';
import { KextStatusRow } from '../components/build/KextStatusRow';
import type { TaskUpdate } from '../bridge/types';
import { Hammer, AlertCircle, RotateCcw } from 'lucide-react';

const pageVariants = {
  hidden: { opacity: 0, y: 10 },
  show: {
    opacity: 1,
    y: 0,
    transition: {
      duration: 0.28,
      staggerChildren: 0.06,
    },
  },
};

const itemVariants = {
  hidden: { opacity: 0, y: 8 },
  show: { opacity: 1, y: 0, transition: { duration: 0.24 } },
};

const DEMO_PHASES = [
  { phase: 'opencore', progress: 0.18, message: 'Preparing demo OpenCore package...' },
  { phase: 'kexts', progress: 0.46, message: 'Hydrating cached kext bundle set...' },
  { phase: 'config', progress: 0.76, message: 'Generating config.plist from demo hardware...' },
  { phase: 'validate', progress: 0.96, message: 'Running offline validation pass...' },
] as const;

export default function Build() {
  const { goNext, markCompleted } = useWizard();
  const { hardware, isDemo } = useHardware();
  const { report: compatReport, selectedTargetOs } = useCompatibility();
  const { buildResult, building, error, build, clear, setBuildResult, setError } = useEfi();
  const timeoutIds = useRef<number[]>([]);

  const [currentPhase, setCurrentPhase] = useState<string>('');
  const [progress, setProgress] = useState(0);
  const [phaseMessage, setPhaseMessage] = useState('');
  const [completedPhases, setCompletedPhases] = useState<Set<string>>(new Set());
  const [started, setStarted] = useState(false);
  const [funQuote, setFunQuote] = useState('');

  // Spinning quotes during build
  const quotePool = useMemo(() => getQuotePool(detectPlatform()), []);

  useEffect(() => {
    if (!started || buildResult) return;
    setFunQuote(getRandomQuote(quotePool));
    const interval = setInterval(() => {
      setFunQuote((prev) => getRandomQuote(quotePool, prev));
    }, 4000);
    return () => clearInterval(interval);
  }, [started, buildResult, quotePool]);

  useEffect(
    () => () => {
      timeoutIds.current.forEach((id) => window.clearTimeout(id));
    },
    [],
  );

  const wait = useCallback((ms: number) => {
    return new Promise<void>((resolve) => {
      const id = window.setTimeout(resolve, ms);
      timeoutIds.current.push(id);
    });
  }, []);

  // Task event subscription for build progress
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    const subscribe = async () => {
      unlisten = await onTaskUpdate((update: TaskUpdate) => {
        if (update.kind === 'efi-build' || update.kind === 'efi_build' || update.kind === 'build_efi') {
          if (update.progress != null) {
            setProgress(update.progress);
          }
          if (update.message) {
            setPhaseMessage(update.message);

            const msg = update.message.toLowerCase();
            let phase = '';
            if (msg.includes('opencore')) phase = 'opencore';
            else if (msg.includes('kext')) phase = 'kexts';
            else if (msg.includes('config') || msg.includes('plist')) phase = 'config';
            else if (msg.includes('validat')) phase = 'validate';

            if (phase) {
              setCurrentPhase((prev) => {
                if (prev && prev !== phase) {
                  setCompletedPhases((s) => new Set(s).add(prev));
                }
                return phase;
              });
            }
          }
          if (update.status === 'completed') {
            setCompletedPhases(new Set(['opencore', 'kexts', 'config', 'validate']));
            setCurrentPhase('');
          }
        }
      });
    };
    subscribe();
    return () => unlisten?.();
  }, []);

  // Auto-navigate on build complete
  useEffect(() => {
    if (buildResult && !building) {
      const timeout = setTimeout(() => {
        markCompleted('build');
        goNext();
      }, 1500);
      return () => clearTimeout(timeout);
    }
  }, [buildResult, building, goNext, markCompleted]);

  const targetOs = selectedTargetOs ?? compatReport?.recommendedOs ?? 'macOS Ventura 13';

  const runDemoBuild = useCallback(async () => {
    setError(null);
    for (const [index, step] of DEMO_PHASES.entries()) {
      await wait(index === 0 ? 320 : 520);
      setCurrentPhase((prev) => {
        if (prev && prev !== step.phase) {
          setCompletedPhases((existing) => new Set(existing).add(prev));
        }
        return step.phase;
      });
      setProgress(step.progress);
      setPhaseMessage(step.message);
    }

    await wait(520);
    setCompletedPhases(new Set(['opencore', 'kexts', 'config', 'validate']));
    setCurrentPhase('');
    setProgress(1);
    setPhaseMessage('Demo build ready.');
    setBuildResult(makeDemoBuildResult(targetOs));
  }, [setBuildResult, setError, targetOs, wait]);

  const handleStart = useCallback(() => {
    if (!hardware) return;
    clear();
    setStarted(true);
    setProgress(0);
    setCurrentPhase('opencore');
    setPhaseMessage('');
    setCompletedPhases(new Set());
    if (isDemo) {
      void runDemoBuild();
      return;
    }
    const profile = {
      ...buildProfile(hardware),
      configStrategy: compatReport?.strategy,
    };
    void build(profile, targetOs);
  }, [hardware, targetOs, build, clear, compatReport?.strategy, isDemo, runDemoBuild]);

  const handleRetry = () => {
    timeoutIds.current.forEach((id) => window.clearTimeout(id));
    timeoutIds.current = [];
    clear();
    setStarted(false);
    setProgress(0);
    setCurrentPhase('');
    setPhaseMessage('');
    setCompletedPhases(new Set());
  };

  // No hardware
  if (!hardware) {
    return (
      <EmptyState
        icon={<Hammer size={28} />}
        title="No hardware data"
        description="Go back to Scan to detect your hardware first."
      />
    );
  }

  // Error
  if (error) {
    return (
      <motion.div
        className="flex flex-col items-center py-24"
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
      >
        <AlertCircle size={32} className="text-[--color-red-5] mb-4" />
        <p className="text-sm text-[--text-primary] mb-2">Build failed</p>
        <p className="text-[0.75rem] text-[--text-tertiary] mb-6 max-w-md text-center">{error}</p>
        <Button variant="secondary" size="sm" onClick={handleRetry} leadingIcon={<RotateCcw size={13} />}>
          Retry
        </Button>
      </motion.div>
    );
  }

  // Build complete
  if (buildResult) {
    const failedKexts = buildResult.kexts.filter((k) => k.status === 'failed');

    return (
      <motion.div variants={pageVariants} initial="hidden" animate="show">
        <motion.h2 variants={itemVariants} className="text-xl font-semibold text-[--text-primary] mb-1">
          Build Complete
        </motion.h2>
        <motion.p variants={itemVariants} className="text-sm text-[--text-tertiary] mb-6">
          OpenCore {buildResult.opencoreVersion} EFI generated successfully.
        </motion.p>

        <motion.p variants={itemVariants} className="text-[0.75rem] text-[--text-tertiary] mb-4">
          Target: {formatMacOsLabel(targetOs)}
        </motion.p>

        {isDemo && (
          <motion.div
            variants={itemVariants}
            className="flex items-center justify-between rounded-lg border border-[--color-blue-3] bg-[--color-blue-1] px-4 py-3 mb-4"
          >
            <div>
              <p className="text-[0.8125rem] text-[--color-blue-7] font-medium">Demo build complete</p>
              <p className="text-[0.6875rem] text-[--color-blue-6] mt-0.5">
                Generated from a local profile without invoking the Rust backend.
              </p>
            </div>
            <Badge variant="info" size="sm" dot>
              Offline
            </Badge>
          </motion.div>
        )}

        {buildResult.warnings.map((w, i) => (
          <motion.div key={i} variants={itemVariants}>
            <WarningBanner variant="warning" message={w} className="mb-3" dismissible />
          </motion.div>
        ))}

        {buildResult.kexts.length > 0 && (
          <motion.div
            variants={itemVariants}
            className="rounded-lg border border-[--border-subtle] bg-[--surface-1] divide-y divide-[--border-subtle] mb-4 overflow-hidden"
          >
            <div className="px-4 py-2.5 flex items-center justify-between">
              <p className="text-[0.6875rem] font-medium text-[--text-tertiary] uppercase tracking-wide">
                Kexts ({buildResult.kexts.length})
              </p>
              {failedKexts.length > 0 && (
                <Badge variant="warning" size="sm">{failedKexts.length} failed</Badge>
              )}
            </div>
            {buildResult.kexts.map((k, i) => (
              <KextStatusRow key={i} kext={k} />
            ))}
          </motion.div>
        )}

        <motion.p variants={itemVariants} className="text-[0.6875rem] text-[--text-tertiary] mb-4">
          Navigating to review...
        </motion.p>
      </motion.div>
    );
  }

  // Pre-start
  if (!started) {
    return (
      <motion.div variants={pageVariants} initial="hidden" animate="show">
        <motion.h2 variants={itemVariants} className="text-xl font-semibold text-[--text-primary] mb-1">
          Build EFI
        </motion.h2>
        <motion.p variants={itemVariants} className="text-sm text-[--text-tertiary] mb-6">
          Generate OpenCore configuration and download required kexts for macOS {targetOs}.
        </motion.p>

        {isDemo && (
          <motion.div
            variants={itemVariants}
            className="flex items-center justify-between rounded-lg border border-[--color-blue-3] bg-[--color-blue-1] px-4 py-3 mb-4"
          >
            <div>
              <p className="text-[0.8125rem] text-[--color-blue-7] font-medium">Demo mode build preview</p>
              <p className="text-[0.6875rem] text-[--color-blue-6] mt-0.5">
                Uses saved artifacts and mocked progress so you can test the full wizard on macOS.
              </p>
            </div>
            <Badge variant="info" size="sm" dot>
              Demo
            </Badge>
          </motion.div>
        )}

        <motion.div variants={itemVariants} className="rounded-lg border border-[--border-subtle] bg-[--surface-1] p-5 mb-8">
          <p className="text-[0.8125rem] text-[--text-secondary] mb-3">Build will:</p>
          <ul className="flex flex-col gap-1.5">
            {[
              'Download the latest OpenCore release',
              'Fetch required kexts for your hardware',
              'Generate a tailored config.plist',
              'Validate the final EFI structure',
            ].map((item, i) => (
              <li key={i} className="flex items-center gap-2 text-[0.8125rem] text-[--text-tertiary]">
                <span className="inline-block size-1 rounded-full bg-[--text-tertiary]" aria-hidden />
                {item}
              </li>
            ))}
          </ul>
        </motion.div>

        <motion.div variants={itemVariants} className="flex justify-end">
          <motion.div whileHover={{ scale: 1.02 }} whileTap={{ scale: 0.98 }}>
            <Button variant="primary" size="md" onClick={handleStart} leadingIcon={<Hammer size={14} />}>
              Start Build
            </Button>
          </motion.div>
        </motion.div>
      </motion.div>
    );
  }

  // Building in progress
  const phases = ['opencore', 'kexts', 'config', 'validate'];

  return (
    <motion.div variants={pageVariants} initial="hidden" animate="show">
      <motion.h2 variants={itemVariants} className="text-xl font-semibold text-[--text-primary] mb-1">
        Building EFI
      </motion.h2>
      <motion.p variants={itemVariants} className="text-sm text-[--text-tertiary] mb-6">
        Generating OpenCore configuration for macOS {targetOs}...
      </motion.p>

      <motion.div variants={itemVariants}>
        <Progress value={progress * 100} variant="accent" height={3} label="Build progress" className="mb-3" />
      </motion.div>

      <motion.div variants={itemVariants} className="flex items-center justify-between mb-3">
        <div className="flex items-center gap-2">
          <motion.span
            className="size-2 rounded-full bg-[--accent]"
            animate={{ scale: [1, 1.35, 1] }}
            transition={{ repeat: Infinity, duration: 1.2, ease: 'easeInOut' }}
          />
          <p className="text-[0.75rem] text-[--text-secondary]">{phaseMessage || 'Initializing build pipeline...'}</p>
        </div>
        <Badge variant="info" size="sm">
          {Math.round(progress * 100)}%
        </Badge>
      </motion.div>

      {/* Fun quote */}
      <AnimatePresence mode="wait">
        <motion.p
          key={funQuote}
          initial={{ opacity: 0, y: 4 }}
          animate={{ opacity: 1, y: 0 }}
          exit={{ opacity: 0, y: -4 }}
          transition={{ duration: 0.3 }}
          className="text-[0.6875rem] text-[--text-tertiary] italic mb-6 min-h-[1.25rem]"
        >
          {funQuote}
        </motion.p>
      </AnimatePresence>

      <motion.div
        variants={itemVariants}
        className="rounded-lg border border-[--border-subtle] bg-[--surface-1] divide-y divide-[--border-subtle] mb-8 overflow-hidden"
      >
        <AnimatePresence initial={false}>
          {phases.map((phase) => (
            <motion.div
              key={phase}
              initial={{ opacity: 0, x: -8 }}
              animate={{ opacity: 1, x: 0 }}
              exit={{ opacity: 0, x: 8 }}
              transition={{ type: 'spring', stiffness: 260, damping: 24 }}
            >
              <BuildPhaseRow
                phase={phase}
                active={currentPhase === phase}
                completed={completedPhases.has(phase)}
                message={currentPhase === phase ? phaseMessage : undefined}
              />
            </motion.div>
          ))}
        </AnimatePresence>
      </motion.div>
    </motion.div>
  );
}
