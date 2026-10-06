import { toFraction } from '../../lib/format';

export type ProgressTone = 'accent' | 'success' | 'danger';

const FILL: Record<ProgressTone, string> = {
  accent: 'bg-accent',
  success: 'bg-ok',
  danger: 'bg-err',
};

/** `value` is a 0..1 fraction; null/NaN shows an indeterminate bar. */
export function Progress({
  value,
  tone = 'accent',
  label,
  className = '',
}: {
  value: number | null | undefined;
  tone?: ProgressTone;
  label?: string;
  className?: string;
}) {
  const fraction = toFraction(value);
  return (
    <div
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={fraction === null ? undefined : Math.round(fraction * 100)}
      className={`relative h-1.5 w-full overflow-hidden rounded-full bg-panel-3 ${className}`}
    >
      {fraction === null ? (
        <span className={`absolute inset-y-0 left-0 w-2/5 rounded-full animate-indeterminate ${FILL[tone]}`} />
      ) : (
        <span
          className={`absolute inset-y-0 left-0 rounded-full transition-[width] duration-300 ease-out ${FILL[tone]}`}
          style={{ width: `${fraction * 100}%` }}
        />
      )}
    </div>
  );
}
