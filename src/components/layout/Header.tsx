import { useState } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { Minus, Square, X } from 'lucide-react';
import { useT } from '../../i18n';
import { useHardware } from '../../stores/hardware';
import { useWizard } from '../../stores/wizard';
import { Badge } from '../ui/Badge';
import { Button } from '../ui/Button';
import { Modal } from '../ui/Modal';

function windowAction(action: 'minimize' | 'toggleMaximize' | 'close') {
  try {
    const win = getCurrentWindow();
    void win[action]().catch(() => undefined);
  } catch {
    // Not inside the desktop runtime.
  }
}

export default function Header() {
  const t = useT();
  const isDemo = useHardware((s) => s.isDemo);
  const locks = useWizard((s) => s.locks);
  const [confirmClose, setConfirmClose] = useState(false);

  const button = 'flex h-8 w-10 items-center justify-center rounded text-fg-3 transition-colors';
  const flashing = locks.includes('flash');

  const requestClose = () => {
    if (locks.length > 0) setConfirmClose(true);
    else windowAction('close');
  };

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

      <Modal
        open={confirmClose}
        onClose={() => setConfirmClose(false)}
        title={t('header.closeRunningTitle')}
        footer={
          <>
            <Button variant="primary" onClick={() => setConfirmClose(false)}>
              {t('header.keepOpen')}
            </Button>
            <Button variant="danger" onClick={() => windowAction('close')}>
              {t('header.closeAnyway')}
            </Button>
          </>
        }
      >
        <p className="text-base text-fg-2">{flashing ? t('header.closeWhileFlashing') : t('header.closeRunningBody')}</p>
      </Modal>
    </header>
  );
}
