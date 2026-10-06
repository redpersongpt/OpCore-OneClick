import { useMemo, useState } from 'react';
import { Bug, Check, ChevronDown, ChevronRight, Copy, Search } from 'lucide-react';
import {
  entryText,
  TROUBLE_CATEGORIES,
  TROUBLESHOOTING,
  type TroubleCategory,
  type TroubleContext,
  type TroubleEntry,
} from '../content/troubleshoot';
import { Badge } from '../components/ui/Badge';
import { Button } from '../components/ui/Button';
import { Modal } from '../components/ui/Modal';
import { useI18n } from '../i18n';
import { collectDiagnostics } from '../lib/diagnostics';
import { copyText, openExternal } from '../lib/external';
import { buildIssueUrl } from '../lib/issue';
import { useApp } from '../stores/app';
import { useBuild } from '../stores/build';
import { useCompat } from '../stores/compat';
import { useHardware } from '../stores/hardware';

type Filter = TroubleCategory | 'all' | 'relevant';

export default function Troubleshoot() {
  const { t, lang } = useI18n();
  const open = useApp((s) => s.troubleshootOpen);
  const setOpen = useApp((s) => s.openTroubleshoot);
  const info = useApp((s) => s.info);
  const profile = useHardware((s) => s.profile);
  const target = useCompat((s) => s.target);

  const [query, setQuery] = useState('');
  const [filter, setFilter] = useState<Filter>('all');
  const [expanded, setExpanded] = useState<string | null>(null);
  const [copied, setCopied] = useState<string | null>(null);
  const [reporting, setReporting] = useState(false);

  const ctx: TroubleContext | null = profile
    ? {
        vendor: profile.cpu.vendor,
        platform: profile.cpu.platform,
        hybrid: profile.cpu.isHybrid,
        chipset: profile.chipset,
        target,
      }
    : target
      ? { vendor: null, platform: null, hybrid: false, chipset: null, target }
      : null;

  const isRelevant = (entry: TroubleEntry) => (ctx && entry.relevant ? entry.relevant(ctx) : false);

  const entries = useMemo(() => {
    const q = query.trim().toLowerCase();
    return TROUBLESHOOTING.filter((entry) => {
      if (filter === 'relevant' && !(ctx && entry.relevant?.(ctx))) return false;
      if (filter !== 'all' && filter !== 'relevant' && entry.category !== filter) return false;
      if (!q) return true;
      const text = entry.text[lang];
      return [text.title, ...text.symptoms, ...text.causes, ...text.fixes, ...(text.advanced ?? []), ...(entry.kexts ?? [])]
        .join('\n')
        .toLowerCase()
        .includes(q);
    });
  }, [query, filter, lang, ctx]);

  const report = async () => {
    setReporting(true);
    try {
      const diagnostics = await collectDiagnostics({
        info,
        profile,
        report: useCompat.getState().report,
        result: useBuild.getState().result,
      });
      await openExternal(
        buildIssueUrl({ title: t('settings.issueTitle'), description: t('settings.issueDescription'), diagnostics }),
      );
    } finally {
      setReporting(false);
    }
  };

  const chips: { id: Filter; label: string }[] = [
    { id: 'all', label: t('trouble.all') },
    ...(ctx ? [{ id: 'relevant' as Filter, label: t('trouble.relevant') }] : []),
    ...TROUBLE_CATEGORIES.map((c) => ({ id: c as Filter, label: t(`trouble.category.${c}`) })),
  ];

  return (
    <Modal
      open={open}
      onClose={() => setOpen(false)}
      width="max-w-3xl"
      title={t('trouble.title')}
      bodyClassName="flex min-h-0 flex-col"
      footer={
        <div className="flex w-full items-center justify-between">
          <Button size="sm" icon={<Bug />} onClick={() => void report()} loading={reporting}>
            {t('settings.report')}
          </Button>
          <Button onClick={() => setOpen(false)}>{t('common.close')}</Button>
        </div>
      }
    >
      <div className="space-y-3 border-b border-line px-5 py-3">
        <div className="relative">
          <Search size={14} className="absolute top-1/2 left-2.5 -translate-y-1/2 text-fg-3" aria-hidden />
          <input
            type="search"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={t('trouble.search')}
            aria-label={t('trouble.search')}
            className="h-8 w-full rounded-md border border-line bg-panel-2 pr-3 pl-8 text-base text-fg placeholder:text-fg-4 focus:border-accent focus:outline-none"
          />
        </div>
        <div className="flex flex-wrap gap-1.5">
          {chips.map((chip) => (
            <button
              key={chip.id}
              type="button"
              aria-pressed={filter === chip.id}
              onClick={() => setFilter(chip.id)}
              className={`h-6 rounded-full px-2.5 text-xs font-medium transition-colors ${
                filter === chip.id ? 'bg-accent text-white' : 'bg-panel-2 text-fg-3 hover:text-fg-2'
              }`}
            >
              {chip.label}
            </button>
          ))}
        </div>
      </div>

      <div className="min-h-0 flex-1 space-y-2 overflow-y-auto px-5 py-3">
        {entries.length === 0 && <p className="py-10 text-center text-sm text-fg-3">{t('trouble.none')}</p>}
        {entries.map((entry) => {
          const text = entry.text[lang];
          const isOpen = expanded === entry.id;
          return (
            <div key={entry.id} className="overflow-hidden rounded-md border border-line bg-panel">
              <button
                type="button"
                aria-expanded={isOpen}
                onClick={() => setExpanded(isOpen ? null : entry.id)}
                className="flex w-full items-center gap-3 px-4 py-2.5 text-left hover:bg-panel-2"
              >
                {isOpen ? <ChevronDown size={14} className="text-fg-3" aria-hidden /> : <ChevronRight size={14} className="text-fg-3" aria-hidden />}
                <span className="flex-1 text-base font-medium text-fg">{text.title}</span>
                {isRelevant(entry) && <Badge tone="info">{t('trouble.relevantBadge')}</Badge>}
                <Badge>{t(`trouble.category.${entry.category}`)}</Badge>
              </button>
              {isOpen && (
                <div className="space-y-3 border-t border-line px-4 py-3">
                  <List title={t('trouble.symptoms')} items={text.symptoms} />
                  <List title={t('trouble.causes')} items={text.causes} />
                  <List title={t('trouble.fixes')} items={text.fixes} ordered />
                  {text.advanced && text.advanced.length > 0 && <List title={t('trouble.advanced')} items={text.advanced} />}
                  {entry.kexts && (
                    <div className="flex flex-wrap items-center gap-1.5">
                      <span className="text-xs text-fg-3">{t('trouble.kexts')}:</span>
                      {entry.kexts.map((k) => (
                        <Badge key={k} tone="info">
                          {k}
                        </Badge>
                      ))}
                    </div>
                  )}
                  <div className="flex justify-end">
                    <Button
                      size="sm"
                      variant="ghost"
                      icon={copied === entry.id ? <Check /> : <Copy />}
                      onClick={async () => {
                        if (await copyText(entryText(entry, text))) {
                          setCopied(entry.id);
                          window.setTimeout(() => setCopied(null), 1500);
                        }
                      }}
                    >
                      {copied === entry.id ? t('common.copied') : t('common.copy')}
                    </Button>
                  </div>
                </div>
              )}
            </div>
          );
        })}
      </div>
    </Modal>
  );
}

function List({ title, items, ordered = false }: { title: string; items: string[]; ordered?: boolean }) {
  const Tag = ordered ? 'ol' : 'ul';
  return (
    <div>
      <p className="mb-1 text-xs font-semibold tracking-wide text-fg-3 uppercase">{title}</p>
      <Tag className={`space-y-0.5 pl-4 text-sm text-fg-2 ${ordered ? 'list-decimal' : 'list-disc'}`}>
        {items.map((item) => (
          <li key={item}>{item}</li>
        ))}
      </Tag>
    </div>
  );
}
