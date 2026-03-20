import { motion } from 'motion/react';
import { AlertTriangle, ArrowRight, Sparkles } from 'lucide-react';
import type { CompatibilityReport } from '../../../electron/compatibility';
import type { CompatibilityMatrix } from '../../../electron/compatibilityMatrix';
import { getMacOSPalette } from '../../lib/macosPalette';
import CompatibilityMatrixView from '../CompatibilityMatrix';

interface Props {
  report: CompatibilityReport;
  matrix: CompatibilityMatrix;
  selectedVersion: string;
  onSelect: (version: string) => void;
  onUseRecommendedVersion?: () => void;
}

export default function VersionStep({
  report,
  matrix,
  selectedVersion,
  onSelect,
  onUseRecommendedVersion,
}: Props) {
  const selectedRow = matrix.rows.find((row) => row.versionName === selectedVersion) ?? null;
  const recommendedRow = matrix.rows.find((row) => row.versionName === matrix.recommendedVersion) ?? null;
  const alternativeRows = matrix.rows.filter((row) => row.versionName !== recommendedRow?.versionName);
  const recommendedPalette = recommendedRow ? getMacOSPalette(recommendedRow.versionName) : null;

  return (
    <div className="space-y-7 pb-8">
      <div className="space-y-2">
        <h2 className="text-4xl font-bold text-white">Choose macOS</h2>
        <p className="max-w-2xl text-sm font-medium leading-relaxed text-white/55">
          Start with the strongest real-world target for this hardware. Older or riskier versions stay visible below, but the best first build is highlighted up front.
        </p>
      </div>

      {recommendedRow && recommendedPalette && (
        <div className={`relative overflow-hidden rounded-[28px] border p-6 shadow-[0_24px_80px_rgba(37,99,235,0.18)] ${recommendedPalette.heroClassName}`}>
          <div className={`pointer-events-none absolute inset-x-0 top-0 h-36 bg-gradient-to-br ${recommendedPalette.fieldClassName}`} />
          <div className={`pointer-events-none absolute right-[-56px] top-[-48px] h-44 w-44 rounded-full blur-3xl ${recommendedPalette.glowClassName}`} />
          <div className="pointer-events-none absolute inset-x-0 top-0 h-px bg-white/22" />
          <div className="flex flex-col gap-5 lg:flex-row lg:items-end lg:justify-between">
            <div className="space-y-4">
              <div className="inline-flex items-center gap-2 rounded-full border border-white/10 bg-black/18 px-3 py-1 text-[10px] font-black uppercase tracking-[0.2em] text-white/78">
                <Sparkles className="h-3.5 w-3.5" />
                Recommended Starting Point
              </div>
              <div className="space-y-2">
                <div className="text-[10px] font-bold uppercase tracking-[0.2em] text-white/38">
                  {recommendedPalette.tone}
                </div>
                <div className="text-3xl font-black tracking-tight text-white">{recommendedRow.versionName}</div>
                <div className="max-w-2xl text-sm leading-relaxed text-white/72">
                  {recommendedRow.reason}
                </div>
              </div>
              <div className="flex flex-wrap items-center gap-2 text-[11px]">
                <span className="rounded-full border border-white/10 bg-white/7 px-2.5 py-1 font-bold uppercase tracking-[0.16em] text-white/65">
                  {recommendedRow.status}
                </span>
                <span className="rounded-full border border-white/10 bg-black/18 px-2.5 py-1 text-white/50">
                  Best first build before you branch into extra fixes
                </span>
              </div>
            </div>

            <div className="flex flex-col gap-2.5 lg:min-w-[240px]">
              {onUseRecommendedVersion && selectedVersion !== recommendedRow.versionName && (
                <button
                  onClick={onUseRecommendedVersion}
                  className={`inline-flex items-center justify-center gap-2 rounded-2xl bg-white px-4 py-3 text-sm font-bold text-slate-950 transition-all ${recommendedPalette.buttonHoverClassName}`}
                >
                  Use Recommended Version
                  <ArrowRight className="h-4 w-4" />
                </button>
              )}
              <div className="rounded-2xl border border-white/10 bg-black/20 px-4 py-3 text-xs leading-relaxed text-white/58">
                {selectedRow?.versionName === recommendedRow.versionName
                  ? 'You are already on the recommended path for a first build.'
                  : `Current selection: ${selectedVersion}. Move to the recommended target first if you want the cleanest troubleshooting path.`}
              </div>
            </div>
          </div>
        </div>
      )}

      {report.warnings.length > 0 && (
        <div className="rounded-2xl border border-amber-500/10 bg-amber-500/5 p-4 space-y-1.5">
          {report.warnings.map((w, i) => (
            <div key={i} className="flex items-start gap-2 text-xs text-amber-400/80">
              <AlertTriangle className="w-3.5 h-3.5 mt-0.5 flex-shrink-0" />
              <span>{w}</span>
            </div>
          ))}
        </div>
      )}

      {!report.isCompatible && (
        <div className="p-5 rounded-2xl bg-red-500/8 border border-red-500/20 text-sm text-red-300/80">
          {report.errors.map((e, i) => <p key={i}>{e}</p>)}
        </div>
      )}

      {selectedRow?.status === 'blocked' && recommendedRow && (
        <div className="rounded-2xl border border-amber-500/20 bg-amber-500/8 p-5 space-y-3">
          <div className="text-[11px] font-black uppercase tracking-[0.2em] text-amber-300/75">
            Selected Version Blocked
          </div>
          <p className="text-sm leading-relaxed text-white/80">
            {selectedRow.reason}
          </p>
          <p className="text-sm leading-relaxed text-white/60">
            Best usable alternative: <span className="font-bold text-white">{recommendedRow.versionName}</span>
          </p>
          {onUseRecommendedVersion && (
            <button
              onClick={onUseRecommendedVersion}
              className="rounded-2xl bg-amber-500 px-4 py-2.5 text-sm font-bold text-black transition-all hover:bg-amber-400"
            >
              Use Recommended Version
            </button>
          )}
        </div>
      )}

      <motion.div
        initial={{ opacity: 0, y: 14 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ delay: 0.05 }}
        className="space-y-3"
      >
        <div className="text-[10px] font-bold uppercase tracking-[0.2em] text-white/32">
          {recommendedRow ? 'Other Available Targets' : 'Available Targets'}
        </div>
        {recommendedRow ? (
          alternativeRows.length > 0 && (
            <CompatibilityMatrixView
              rows={alternativeRows}
              selectedVersion={selectedVersion}
              onSelect={onSelect}
            />
          )
        ) : (
          <CompatibilityMatrixView
            rows={matrix.rows}
            selectedVersion={selectedVersion}
            onSelect={onSelect}
          />
        )}
      </motion.div>
    </div>
  );
}
