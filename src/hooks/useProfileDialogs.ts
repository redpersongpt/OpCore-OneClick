import { useState } from 'react';
import { open, save } from '@tauri-apps/plugin-dialog';
import { useT } from '../i18n';
import { importProfile } from '../stores/flow';
import { useHardware } from '../stores/hardware';

/** Open/save dialogs for hardware profiles (`import_profile` / `export_profile`). */
export function useProfileDialogs() {
  const t = useT();
  const filters = [{ name: t('hardware.profileFile'), extensions: ['json'] }];
  const [busy, setBusy] = useState<'import' | 'export' | null>(null);
  const [exportedTo, setExportedTo] = useState<string | null>(null);

  const doImport = async (): Promise<boolean> => {
    setBusy('import');
    try {
      const picked = await open({ multiple: false, directory: false, filters });
      if (typeof picked !== 'string') return false;
      return await importProfile(picked);
    } catch {
      return false;
    } finally {
      setBusy(null);
    }
  };

  const doExport = async (): Promise<boolean> => {
    const profile = useHardware.getState().profile;
    if (!profile) return false;
    setBusy('export');
    setExportedTo(null);
    try {
      const name = (profile.motherboardModel || profile.cpu.name || 'hardware').replace(/[^\w.-]+/g, '-').slice(0, 40);
      const target = await save({ defaultPath: `${name}-profile.json`, filters });
      if (!target) return false;
      const ok = await useHardware.getState().exportTo(target);
      if (ok) setExportedTo(target);
      return ok;
    } catch {
      return false;
    } finally {
      setBusy(null);
    }
  };

  return { busy, exportedTo, doImport, doExport };
}
