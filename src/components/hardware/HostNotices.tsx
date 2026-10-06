import { useState, type ReactNode } from 'react';
import { Check, Copy, FileInput, PencilLine } from 'lucide-react';
import { useT } from '../../i18n';
import { copyText } from '../../lib/external';
import { LINUX_ROOT_COMMANDS, type ScanNotice } from '../../lib/host';
import { Banner } from '../ui/Banner';
import { Button } from '../ui/Button';

/** Ways to describe another PC than the one the app runs on. */
export function OtherPcActions({
  onImport,
  onManual,
  importing,
}: {
  onImport: () => void;
  onManual: () => void;
  importing: boolean;
}) {
  const t = useT();
  return (
    <>
      <Button size="sm" variant="primary" icon={<FileInput />} onClick={onImport} loading={importing}>
        {t('scan.import')}
      </Button>
      <Button size="sm" icon={<PencilLine />} onClick={onManual}>
        {t('scan.manual')}
      </Button>
    </>
  );
}

/** What the scan of this host can and cannot tell, with the way forward. */
export function HostNotices({
  notices,
  onImport,
  onManual,
  importing = false,
  compact = false,
}: {
  notices: readonly ScanNotice[];
  onImport: () => void;
  onManual: () => void;
  importing?: boolean;
  /** Short form for the hardware step (the scan step shows the full instructions). */
  compact?: boolean;
}) {
  const t = useT();
  const actions = <OtherPcActions onImport={onImport} onManual={onManual} importing={importing} />;
  return (
    <>
      {notices.map((notice) => {
        switch (notice) {
          case 'apple_silicon':
            return (
              <Banner key={notice} tone="danger" title={t('host.appleSilicon.title')} actions={actions}>
                <p>{t('host.appleSilicon.body')}</p>
                <p className="mt-1">{t('host.otherPc')}</p>
              </Banner>
            );
          case 'apple_profile':
            return (
              <Banner key={notice} tone="danger" title={t('host.appleProfile.title')} actions={actions}>
                <p>{t('host.appleProfile.body')}</p>
              </Banner>
            );
          case 'real_mac':
            return (
              <Banner key={notice} tone="warning" title={t('host.realMac.title')} actions={actions}>
                <p>{t('host.realMac.body')}</p>
                <p className="mt-1">{t('host.otherPc')}</p>
              </Banner>
            );
          case 'hackintosh':
            return (
              <Banner key={notice} tone="warning" title={t('host.hackintosh.title')} actions={compact ? undefined : actions}>
                <p>{t('host.hackintosh.body')}</p>
              </Banner>
            );
          case 'linux_acpi':
            return (
              <Banner key={notice} tone="warning" title={t('host.linuxAcpi.title')}>
                <p>{t('host.linuxAcpi.body')}</p>
                {!compact && (
                  <div className="mt-2 space-y-1.5">
                    <p>{t('host.linuxAcpi.how')}</p>
                    <Command label={t('host.linuxAcpi.deb')} command={LINUX_ROOT_COMMANDS.deb} />
                    <Command label={t('host.linuxAcpi.appImage')} command={LINUX_ROOT_COMMANDS.appImage} />
                    <Command label={t('host.linuxAcpi.wayland')} command={LINUX_ROOT_COMMANDS.wayland} />
                    <p>{t('host.linuxAcpi.tip')}</p>
                    <p>{t('host.linuxAcpi.without')}</p>
                  </div>
                )}
              </Banner>
            );
          default:
            return null;
        }
      })}
    </>
  );
}

function Command({ label, command }: { label: ReactNode; command: string }) {
  const t = useT();
  const [copied, setCopied] = useState(false);
  return (
    <div>
      <p className="text-xs text-fg-3">{label}</p>
      <div className="mt-0.5 flex items-center gap-2 rounded border border-line bg-bg px-2 py-1">
        <code className="min-w-0 flex-1 truncate font-mono text-xs text-fg">{command}</code>
        <button
          type="button"
          onClick={async () => {
            if (await copyText(command)) {
              setCopied(true);
              window.setTimeout(() => setCopied(false), 1500);
            }
          }}
          aria-label={t('common.copy')}
          className="shrink-0 rounded p-0.5 text-fg-3 hover:bg-panel-2 hover:text-fg"
        >
          {copied ? <Check size={12} aria-hidden /> : <Copy size={12} aria-hidden />}
        </button>
      </div>
    </div>
  );
}
