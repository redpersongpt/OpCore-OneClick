import { forwardRef, type ButtonHTMLAttributes, type ReactNode } from 'react';
import { Loader2 } from 'lucide-react';

export type ButtonVariant = 'primary' | 'secondary' | 'danger' | 'ghost';
export type ButtonSize = 'sm' | 'md';

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  loading?: boolean;
  icon?: ReactNode;
}

const VARIANTS: Record<ButtonVariant, string> = {
  primary: 'bg-fg text-bg hover:bg-white active:bg-fg-2',
  secondary: 'bg-panel-2 text-fg-2 border border-line hover:bg-panel-3 hover:text-fg hover:border-line-strong',
  danger: 'bg-err-soft text-err-fg border border-err-line hover:bg-err-line/50',
  ghost: 'text-fg-3 hover:bg-panel-2 hover:text-fg',
};

const SIZES: Record<ButtonSize, string> = {
  sm: 'h-7 px-2.5 gap-1.5 text-sm',
  md: 'h-8 px-3.5 gap-2 text-base',
};

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  { variant = 'secondary', size = 'md', loading = false, icon, disabled, className = '', children, type, ...props },
  ref,
) {
  return (
    <button
      ref={ref}
      type={type ?? 'button'}
      disabled={disabled || loading}
      aria-busy={loading || undefined}
      className={[
        'inline-flex shrink-0 items-center justify-center rounded-md font-medium whitespace-nowrap select-none',
        'transition-colors duration-150 disabled:cursor-not-allowed disabled:opacity-40',
        VARIANTS[variant],
        SIZES[size],
        className,
      ].join(' ')}
      {...props}
    >
      {loading ? (
        <Loader2 size={14} className="animate-spin" aria-hidden />
      ) : icon ? (
        <span className="flex items-center [&>svg]:size-3.5" aria-hidden>
          {icon}
        </span>
      ) : null}
      {children}
    </button>
  );
});
