import type { ReactNode } from 'react';

export function Section({
  title,
  description,
  actions,
  children,
  className = '',
  flush = false,
}: {
  title?: ReactNode;
  description?: ReactNode;
  actions?: ReactNode;
  children?: ReactNode;
  className?: string;
  /** Children render edge to edge (lists with their own dividers). */
  flush?: boolean;
}) {
  return (
    <section className={`rounded-lg border border-line bg-panel ${className}`}>
      {(title || actions) && (
        <header className="flex items-start justify-between gap-3 border-b border-line px-4 py-3">
          <div className="min-w-0">
            {title && <h3 className="text-base font-semibold text-fg">{title}</h3>}
            {description && <p className="mt-0.5 text-sm text-fg-3">{description}</p>}
          </div>
          {actions && <div className="flex shrink-0 items-center gap-2">{actions}</div>}
        </header>
      )}
      {children !== undefined && <div className={flush ? '' : 'px-4 py-3'}>{children}</div>}
    </section>
  );
}

export function KeyValue({ label, children, mono = false }: { label: ReactNode; children: ReactNode; mono?: boolean }) {
  return (
    <div className="flex items-baseline gap-3 py-1.5">
      <span className="w-36 shrink-0 text-sm text-fg-3">{label}</span>
      <span className={`min-w-0 flex-1 break-words text-base text-fg ${mono ? 'font-mono text-sm' : ''}`}>
        {children}
      </span>
    </div>
  );
}

export function PageHeader({ title, subtitle, actions }: { title: ReactNode; subtitle?: ReactNode; actions?: ReactNode }) {
  return (
    <div className="mb-5 flex items-start justify-between gap-4">
      <div className="min-w-0">
        <h1 className="text-xl font-semibold tracking-tight text-fg">{title}</h1>
        {subtitle && <p className="mt-1 text-base text-fg-3">{subtitle}</p>}
      </div>
      {actions && <div className="flex shrink-0 items-center gap-2">{actions}</div>}
    </div>
  );
}

/** Bottom action row of a wizard step. */
export function StepActions({ left, children }: { left?: ReactNode; children?: ReactNode }) {
  return (
    <div className="mt-6 flex items-center justify-between gap-3 border-t border-line pt-4">
      <div className="flex items-center gap-2">{left}</div>
      <div className="flex items-center gap-2">{children}</div>
    </div>
  );
}
