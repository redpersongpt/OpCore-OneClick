import type { ReactNode } from 'react';

export type Tone = 'success' | 'warning' | 'danger' | 'info' | 'neutral';

const TONES: Record<Tone, string> = {
  success: 'bg-ok-soft text-ok-fg border-ok-line',
  warning: 'bg-warn-soft text-warn-fg border-warn-line',
  danger: 'bg-err-soft text-err-fg border-err-line',
  info: 'bg-accent-soft text-accent-fg border-accent-line',
  neutral: 'bg-panel-2 text-fg-2 border-line',
};

const DOTS: Record<Tone, string> = {
  success: 'bg-ok',
  warning: 'bg-warn',
  danger: 'bg-err',
  info: 'bg-accent',
  neutral: 'bg-fg-3',
};

export function Badge({
  tone = 'neutral',
  dot = false,
  className = '',
  title,
  children,
}: {
  tone?: Tone;
  dot?: boolean;
  className?: string;
  title?: string;
  children: ReactNode;
}) {
  return (
    <span
      title={title}
      className={`inline-flex items-center gap-1 rounded border px-1.5 py-0.5 text-2xs font-medium leading-none tracking-wide whitespace-nowrap ${TONES[tone]} ${className}`}
    >
      {dot && <span className={`size-1.5 rounded-full ${DOTS[tone]}`} aria-hidden />}
      {children}
    </span>
  );
}

export function Dot({ tone, className = '' }: { tone: Tone; className?: string }) {
  return <span className={`inline-block size-2 shrink-0 rounded-full ${DOTS[tone]} ${className}`} aria-hidden />;
}
