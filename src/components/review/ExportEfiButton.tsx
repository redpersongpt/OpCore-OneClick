import { useState } from 'react';
import { FolderOutput } from 'lucide-react';
import { open } from '@tauri-apps/plugin-dialog';
import { api } from '../../bridge/api';
import { toAppError, type AppError } from '../../bridge/errors';
import { useT } from '../../i18n';
import { ErrorPanel } from '../feedback/ErrorPanel';
import { Banner } from '../ui/Banner';
import { Button, type ButtonVariant } from '../ui/Button';

/** Copies the built EFI folder to a folder the user picks (`export_efi`). */
export function ExportEfiButton({ efiPath, variant = 'secondary' }: { efiPath: string; variant?: ButtonVariant }) {
  const t = useT();
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState<string | null>(null);
  const [error, setError] = useState<AppError | null>(null);

  const run = async () => {
    setBusy(true);
    setError(null);
    setDone(null);
    try {
      const destination = await open({ directory: true, multiple: false, title: t('export.pick') });
      if (typeof destination !== 'string') return;
      setDone(await api.exportEfi(efiPath, destination));
    } catch (err) {
      setError(toAppError(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="space-y-2">
      <Button variant={variant} icon={<FolderOutput />} onClick={() => void run()} loading={busy}>
        {t('export.button')}
      </Button>
      {done && <Banner tone="success">{t('export.done', { path: done })}</Banner>}
      {error && <ErrorPanel error={error} title={t('export.failed')} compact />}
    </div>
  );
}
