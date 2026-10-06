import { Minus, Square, X } from 'lucide-react';
import { requestClose } from '../../hooks/useCloseGuard';
import { useT } from '../../i18n';
import { windowAction } from '../../lib/window';
import { useHardware } from '../../stores/hardware';
import { Badge } from '../ui/Badge';

export default function Header() {
  const t = useT();
  const isDemo = useHardware((s) => s.isDemo);
  const button = 'flex h-8 w-10 items-center justify-center rounded text-fg-3 transition-colors';

  return (
    <header className="flex h-11 shrink-0 items-center justify-between border-b border-line bg-bg pl-5 pr-2" data-tauri-drag-region>
      <div className="flex items-center gap-2" data-tauri-drag-region>
        {isDemo && (
          <Badge tone="warning" dot>
            {t('header.demo')}
          </Badge>
        )}
      </div>
      <div className="flex items-center gap-0.5">
        <button type="button" aria-label={t('header.minimize')} onClick={() => windowAction('minimize')} className={`${button} hover:bg-panel-2 hover:text-fg-2`}>
          <Minus size={14} aria-hidden />
        </button>
        <button type="button" aria-label={t('header.maximize')} onClick={() => windowAction('toggleMaximize')} className={`${button} hover:bg-panel-2 hover:text-fg-2`}>
          <Square size={12} aria-hidden />
        </button>
        <button type="button" aria-label={t('header.close')} onClick={requestClose} className={`${button} hover:bg-err/15 hover:text-err`}>
          <X size={14} aria-hidden />
        </button>
      </div>
    </header>
  );
}
