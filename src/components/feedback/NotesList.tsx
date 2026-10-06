import { AlertOctagon, AlertTriangle, Info } from 'lucide-react';
import type { NoteLevel, PlanNote } from '../../bridge/types';
import { useT } from '../../i18n';
import { componentLabel } from '../../lib/labels';

const ICON: Record<NoteLevel, { Icon: typeof Info; cls: string }> = {
  info: { Icon: Info, cls: 'text-accent-fg' },
  warning: { Icon: AlertTriangle, cls: 'text-warn' },
  blocking: { Icon: AlertOctagon, cls: 'text-err' },
};

const ORDER: Record<NoteLevel, number> = { blocking: 0, warning: 1, info: 2 };

/** Planner / compatibility notes, most severe first. */
export function NotesList({ notes, empty }: { notes: readonly PlanNote[]; empty?: string }) {
  const t = useT();
  if (notes.length === 0) return empty ? <p className="text-sm text-fg-3">{empty}</p> : null;
  const sorted = [...notes].sort((a, b) => ORDER[a.level] - ORDER[b.level]);
  return (
    <ul className="space-y-2.5">
      {sorted.map((note, index) => {
        const { Icon, cls } = ICON[note.level];
        return (
          <li key={`${note.component}-${note.title}-${index}`} className="flex gap-2.5">
            <Icon size={14} className={`mt-0.5 shrink-0 ${cls}`} aria-label={t(`note.${note.level}`)} />
            <div className="min-w-0">
              <p className="text-base text-fg">
                {note.title}
                {note.component && <span className="ml-2 text-xs text-fg-3 uppercase">{componentLabel(t, note.component)}</span>}
              </p>
              {note.detail && <p className="mt-0.5 text-sm leading-relaxed text-fg-2">{note.detail}</p>}
            </div>
          </li>
        );
      })}
    </ul>
  );
}
