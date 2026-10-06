import { useEffect, useState } from 'react';
import { AlertOctagon } from 'lucide-react';
import type { DiskInfo, FlashConfirmation } from '../../bridge/types';
import { useT } from '../../i18n';
import { confirmPhrase, diskTitle, phraseMatches } from '../../lib/disk';
import { formatBytes } from '../../lib/format';
import { macosLabel } from '../../lib/macos';
import { toNum } from '../../lib/num';
import { Button } from '../ui/Button';
import { Modal } from '../ui/Modal';

const COOLDOWN_MS = 2000;

/**
 * Erase confirmation: the user must type the drive size (plus its device name
 * when another drive has the same size) before the single-use token is spent.
 */
export function FlashConfirmDialog({
  confirmation,
  disk,
  disks,
  onConfirm,
  onCancel,
  onRenew,
  renewing,
}: {
  confirmation: FlashConfirmation | null;
  disk: DiskInfo | null;
  /** Every listed drive, to make the confirmation phrase unambiguous. */
  disks: readonly DiskInfo[];
  onConfirm: () => void;
  onCancel: () => void;
  onRenew: () => void;
  renewing: boolean;
}) {
  const t = useT();
  const [typed, setTyped] = useState('');
  const [now, setNow] = useState(() => Date.now());
  const [openedAt, setOpenedAt] = useState(() => Date.now());
  const token = confirmation?.token ?? null;

  useEffect(() => {
    if (!token) return;
    setTyped('');
    setOpenedAt(Date.now());
    setNow(Date.now());
    const timer = window.setInterval(() => setNow(Date.now()), 500);
    return () => window.clearInterval(timer);
  }, [token]);

  // Never show (or spend) a token issued for another drive than the one on screen.
  const open = confirmation !== null && disk !== null && confirmation.device === disk.devicePath;
  const phrase = disk ? confirmPhrase(disk, disks) : '';
  const withName = disk !== null && phrase !== disk.sizeDisplay.trim();
  const recovery = confirmation?.recovery ?? null;
  const expiresAt = toNum(confirmation?.expiresAt) ?? 0;
  const secondsLeft = Math.max(0, Math.ceil((expiresAt - now) / 1000));
  const expired = open && secondsLeft === 0;
  const cooling = now - openedAt < COOLDOWN_MS;
  const matches = phraseMatches(typed, phrase);
  const canConfirm = open && matches && !expired && !cooling;

  return (
    <Modal
      open={open}
      onClose={onCancel}
      width="max-w-lg"
      title={
        <span className="flex items-center gap-2 text-err-fg">
          <AlertOctagon size={15} aria-hidden />
          {t('flash.confirmTitle')}
        </span>
      }
      footer={
        <>
          <Button variant="ghost" onClick={onCancel}>
            {t('common.cancel')}
          </Button>
          {expired ? (
            <Button onClick={onRenew} loading={renewing}>
              {t('flash.renew')}
            </Button>
          ) : (
            <Button variant="danger" onClick={onConfirm} disabled={!canConfirm}>
              {t('flash.confirmButton')}
            </Button>
          )}
        </>
      }
    >
      {open && disk && confirmation && (
        <div className="space-y-4">
          <div className="rounded-md border border-err-line bg-err-soft px-4 py-3">
            <p className="text-base font-medium text-fg">{diskTitle(disk)}</p>
            <p className="font-mono text-xs text-fg-2">
              {confirmation.diskDisplay || disk.devicePath} · {disk.sizeDisplay}
            </p>
            <p className="mt-2 text-sm text-err-fg">{t('flash.eraseWarning')}</p>
            {disk.partitions.length > 0 && (
              <ul className="mt-2 space-y-0.5 text-sm text-fg-2">
                {disk.partitions.map((p) => (
                  <li key={p.number}>
                    #{p.number} {p.label || t('flash.noLabel')} · {p.filesystem ?? '?'} · {formatBytes(toNum(p.sizeBytes))}
                    {p.mountPoint ? ` · ${p.mountPoint}` : ''}
                  </li>
                ))}
              </ul>
            )}
          </div>

          <div className="text-sm text-fg-2">
            <p className="font-medium text-fg">{t('flash.willWrite')}</p>
            <ul className="mt-1 list-disc space-y-0.5 pl-4">
              <li>{t('flash.writeEfi')}</li>
              {recovery && <li>{t('flash.writeRecovery', { version: macosLabel(recovery) })}</li>}
            </ul>
          </div>

          <div className="space-y-1.5">
            <label htmlFor="flash-confirm" className="block text-sm text-fg-2">
              {withName ? t('flash.typeToConfirmName') : t('flash.typeToConfirm')} <span className="font-mono font-semibold text-fg">{phrase}</span>
            </label>
            <input
              id="flash-confirm"
              data-autofocus
              value={typed}
              onChange={(e) => setTyped(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' && canConfirm) onConfirm();
              }}
              autoComplete="off"
              spellCheck={false}
              disabled={expired}
              className={`h-8 w-full rounded-md border bg-panel-2 px-2.5 font-mono text-sm text-fg focus:outline-none ${
                matches ? 'border-ok-line' : 'border-line focus:border-accent'
              }`}
            />
            <p className="text-xs text-fg-3">
              {expired ? t('flash.expired') : t('flash.expiresIn', { seconds: secondsLeft })}
            </p>
          </div>
        </div>
      )}
    </Modal>
  );
}
