import { motion } from 'motion/react';
import { AlertTriangle } from 'lucide-react';
import type { CompatibilityReport } from '../../../electron/compatibility';
import type { CompatibilityMatrix } from '../../../electron/compatibilityMatrix';
import CompatibilityMatrixView from '../CompatibilityMatrix';

interface Props {
  report: CompatibilityReport;
  matrix: CompatibilityMatrix;
  selectedVersion: string;
  onSelect: (version: string) => void;
}

export default function VersionStep({ report, matrix, selectedVersion, onSelect }: Props) {
  return (
    <div className="space-y-6">
      <div>
        <h2 className="text-4xl font-bold text-white mb-2">Choose macOS</h2>
        <p className="text-[#888888] font-medium text-sm">
          Every target is shown here, but blocked versions remain unselectable and inherit the current compatibility logic unchanged.
        </p>
      </div>

      {report.warnings.length > 0 && (
        <div className="p-4 rounded-2xl bg-amber-500/5 border border-amber-500/10 space-y-1.5">
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

      <motion.div
        initial={{ opacity: 0, y: 14 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ delay: 0.05 }}
      >
        <CompatibilityMatrixView
          rows={matrix.rows}
          selectedVersion={selectedVersion}
          onSelect={onSelect}
        />
      </motion.div>
    </div>
  );
}
