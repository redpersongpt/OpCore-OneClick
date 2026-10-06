import type { CompatibilityReport, MacOsVersion } from '../../bridge/types';
import { useT } from '../../i18n';
import { sortedVersions } from '../../lib/compat';
import { macosName } from '../../lib/macos';
import { Badge } from '../ui/Badge';

export function VersionPicker({
  report,
  selected,
  onSelect,
  disabled = false,
}: {
  report: CompatibilityReport;
  selected: MacOsVersion | null;
  onSelect: (version: MacOsVersion) => void;
  disabled?: boolean;
}) {
  const t = useT();
  const versions = sortedVersions(report);

  return (
    <div role="radiogroup" aria-label={t('compat.versions')} className="grid grid-cols-3 gap-2">
      {versions.map((option) => {
        const isSelected = option.version === selected;
        return (
          <button
            key={option.version}
            type="button"
            role="radio"
            aria-checked={isSelected}
            disabled={disabled}
            onClick={() => onSelect(option.version)}
            className={`flex flex-col items-start gap-1.5 rounded-md border px-3 py-2.5 text-left transition-colors disabled:cursor-not-allowed ${
              isSelected
                ? 'border-accent bg-accent-soft'
                : option.supported
                  ? 'border-line bg-panel hover:border-line-strong hover:bg-panel-2'
                  : 'border-line bg-panel opacity-60 hover:opacity-90'
            }`}
          >
            <span className="flex w-full items-baseline justify-between gap-2">
              <span className="text-base font-medium text-fg">{macosName(option.version)}</span>
              <span className="font-mono text-xs text-fg-3">{option.version}</span>
            </span>
            <span className="flex flex-wrap gap-1">
              {option.recommended && <Badge tone="info">{t('compat.recommended')}</Badge>}
              {option.supported ? (
                <Badge tone="success">{t('compat.supported')}</Badge>
              ) : (
                <Badge tone="danger">{t('compat.notSupported')}</Badge>
              )}
              {option.needsRootPatch && <Badge tone="warning">{t('compat.rootPatch')}</Badge>}
            </span>
            {option.notes.length > 0 && (
              <span className="line-clamp-2 text-xs leading-snug text-fg-3">{option.notes[0]}</span>
            )}
          </button>
        );
      })}
    </div>
  );
}
