import type { ReactNode } from 'react';
import { Spinner } from '../ui/Spinner';

export function LoadingState({ message, children }: { message: ReactNode; children?: ReactNode }) {
  return (
    <div className="flex flex-col items-center justify-center gap-3 py-14 text-center animate-fade-in">
      <Spinner size={20} />
      <p className="text-base text-fg-3">{message}</p>
      {children}
    </div>
  );
}

export function EmptyState({
  icon,
  title,
  description,
  action,
}: {
  icon?: ReactNode;
  title: ReactNode;
  description?: ReactNode;
  action?: ReactNode;
}) {
  return (
    <div className="flex flex-col items-center justify-center gap-2 px-6 py-10 text-center">
      {icon && <span className="text-fg-3">{icon}</span>}
      <p className="text-base font-medium text-fg">{title}</p>
      {description && <p className="max-w-sm text-sm text-fg-3">{description}</p>}
      {action && <div className="mt-2">{action}</div>}
    </div>
  );
}
