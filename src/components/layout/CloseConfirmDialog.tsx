import { useT } from '../../i18n';
import { closeNow, closeRisk } from '../../hooks/useCloseGuard';
import { useApp } from '../../stores/app';
import { useTasks } from '../../stores/tasks';
import { useWizard } from '../../stores/wizard';
import { Button } from '../ui/Button';
import { Modal } from '../ui/Modal';

/** Shown when the window is about to close while an operation runs. */
export default function CloseConfirmDialog() {
  const t = useT();
  const open = useApp((s) => s.closeConfirmOpen);
  const setOpen = useApp((s) => s.openCloseConfirm);
  // Re-render when locks or tasks change so the text follows what is running.
  useWizard((s) => s.locks);
  useTasks((s) => s.tasks);
  const flashing = closeRisk() === 'flash';

  return (
    <Modal
      open={open}
      onClose={() => setOpen(false)}
      title={t('header.closeRunningTitle')}
      footer={
        <>
          <Button variant="primary" onClick={() => setOpen(false)}>
            {t('header.keepOpen')}
          </Button>
          <Button variant="danger" onClick={closeNow}>
            {t('header.closeAnyway')}
          </Button>
        </>
      }
    >
      <p className="text-base text-fg-2">{flashing ? t('header.closeWhileFlashing') : t('header.closeRunningBody')}</p>
    </Modal>
  );
}
