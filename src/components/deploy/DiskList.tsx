import { HardDrive, RefreshCw } from 'lucide-react';
import type { DiskInfo } from '../../bridge/types';
import { useT, type Translate } from '../../i18n';
import { diskBlock, diskTitle, type DiskBlock } from '../../lib/disk';
import { formatBytes } from '../../lib/format';
import { toNum } from '../../lib/num';
import { ErrorPanel } from '../feedback/ErrorPanel';
import { EmptyState } from '../feedback/States';
import { Badge } from '../ui/Badge';
import { Button } from '../ui/Button';
import { Section } from '../ui/Section';
import { Spinner } from '../ui/Spinner';
import { useDeploy } from '../../stores/deploy';

function blockText(t: Translate, block: DiskBlock): string {
  switch (block.kind) {
    case 'system':
      return t('disk.blockedSystem');
    case 'backend':
      return block.reason;
    case 'too_small':
      return t('disk.blockedSmall', { size: formatBytes(block.minBytes) });
  }
}

export function DiskList({ minBytes, disabled }: { minBytes: number; disabled: boolean }) {
  const t = useT();
  const disks = useDeploy((s) => s.disks);
  const loading = useDeploy((s) => s.disksLoading);
  const loaded = useDeploy((s) => s.disksLoaded);
  const error = useDeploy((s) => s.disksError);
  const selected = useDeploy((s) => s.selected);
  const select = useDeploy((s) => s.select);
  const refresh = useDeploy((s) => s.refreshDisks);

  return (
    <Section
      title={t('disk.title')}
      description={t('disk.hint')}
      actions={
        <Button size="sm" variant="ghost" icon={<RefreshCw />} onClick={() => void refresh()} loading={loading} disabled={disabled}>
          {t('disk.refresh')}
        </Button>
      }
      flush
    >
      {error ? (
        <div className="p-4">
          <ErrorPanel error={error} title={t('disk.failed')} compact />
        </div>
      ) : !loaded ? (
        <div className="flex items-center gap-2 px-4 py-3 text-sm text-fg-3">
          <Spinner size={14} /> {t('disk.loading')}
        </div>
      ) : disks.length === 0 ? (
        <EmptyState icon={<HardDrive size={20} />} title={t('disk.none')} description={t('disk.noneHint')} />
      ) : (
        <ul role="radiogroup" aria-label={t('disk.title')} className="divide-y divide-line">
          {disks.map((disk) => (
            <DiskRow
              key={disk.devicePath}
              disk={disk}
              block={diskBlock(disk, minBytes)}
              selected={selected === disk.devicePath}
              onSelect={() => select(disk.devicePath)}
              disabled={disabled}
            />
          ))}
        </ul>
      )}
    </Section>
  );
}

function DiskRow({
  disk,
  block,
  selected,
  onSelect,
  disabled,
}: {
  disk: DiskInfo;
  block: DiskBlock | null;
  selected: boolean;
  onSelect: () => void;
  disabled: boolean;
}) {
  const t = useT();
  const unavailable = block !== null;
  return (
    <li>
      <button
        type="button"
        role="radio"
        aria-checked={selected}
        disabled={unavailable || disabled}
        onClick={onSelect}
        className={`flex w-full items-start gap-3 px-4 py-3 text-left transition-colors disabled:cursor-not-allowed ${
          selected ? 'bg-accent-soft' : unavailable ? 'opacity-60' : 'hover:bg-panel-2'
        }`}
      >
        <span
          className={`mt-1 flex size-3.5 shrink-0 items-center justify-center rounded-full border ${
            selected ? 'border-accent' : 'border-line-strong'
          }`}
          aria-hidden
        >
          {selected && <span className="size-1.5 rounded-full bg-accent" />}
        </span>
        <span className="min-w-0 flex-1">
          <span className="flex flex-wrap items-center gap-2">
            <span className="text-base font-medium text-fg">{diskTitle(disk)}</span>
            <span className="text-sm text-fg-2">{disk.sizeDisplay}</span>
            {disk.transport && <Badge>{disk.transport.toUpperCase()}</Badge>}
            {disk.removable && <Badge tone="info">{t('disk.removable')}</Badge>}
            {disk.isSystemDisk && <Badge tone="danger">{t('disk.system')}</Badge>}
          </span>
          <span className="block font-mono text-xs text-fg-3">
            {disk.devicePath}
            {disk.partitionTable ? ` · ${disk.partitionTable.toUpperCase()}` : ''}
          </span>
          {disk.partitions.length > 0 && (
            <span className="mt-1 block text-xs text-fg-3">
              {disk.partitions
                .map((p) =>
                  [p.label || t('flash.noLabel'), p.filesystem, formatBytes(toNum(p.sizeBytes)), p.mountPoint]
                    .filter(Boolean)
                    .join(' · '),
                )
                .join('  |  ')}
            </span>
          )}
          {block && <span className="mt-1 block text-xs text-warn-fg">{blockText(t, block)}</span>}
        </span>
      </button>
    </li>
  );
}
