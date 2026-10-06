import { Loader2 } from 'lucide-react';

export function Spinner({ size = 16, className = '', label }: { size?: number; className?: string; label?: string }) {
  return (
    <span role="status" aria-label={label} className={`inline-flex shrink-0 text-accent ${className}`}>
      <Loader2 size={size} className="animate-spin" aria-hidden />
    </span>
  );
}
