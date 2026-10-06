import type { ReactNode } from 'react';
import { AlertOctagon, AlertTriangle, CheckCircle2, Info } from 'lucide-react';

export type BannerTone = 'info' | 'warning' | 'danger' | 'success';

const STYLES: Record<BannerTone, { box: string; icon: string; Icon: typeof Info }> = {
  info: { box: 'bg-accent-soft border-accent-line', icon: 'text-accent-fg', Icon: Info },
  warning: { box: 'bg-warn-soft border-warn-line', icon: 'text-warn', Icon: AlertTriangle },
  danger: { box: 'bg-err-soft border-err-line', icon: 'text-err', Icon: AlertOctagon },
  success: { box: 'bg-ok-soft border-ok-line', icon: 'text-ok', Icon: CheckCircle2 },
};

export function Banner({
  tone = 'info',
  title,
  children,
  actions,
  className = '',
}: {
  tone?: BannerTone;
  title?: ReactNode;
  children?: ReactNode;
  actions?: ReactNode;
  className?: string;
}) {
  const { box, icon, Icon } = STYLES[tone];
  return (
    <div
      role={tone === 'danger' || tone === 'warning' ? 'alert' : 'status'}
      className={`flex gap-3 rounded-md border px-3.5 py-3 ${box} ${className}`}
    >
      <Icon size={15} className={`mt-px shrink-0 ${icon}`} aria-hidden />
      <div className="min-w-0 flex-1">
        {title && <p className="text-base font-medium text-fg">{title}</p>}
        {children && <div className={`text-sm leading-relaxed text-fg-2 ${title ? 'mt-0.5' : ''}`}>{children}</div>}
        {actions && <div className="mt-2.5 flex flex-wrap gap-2">{actions}</div>}
      </div>
    </div>
  );
}
