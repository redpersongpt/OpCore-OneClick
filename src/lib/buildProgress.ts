/**
 * Structured progress of an EFI build (`TaskUpdate.detail` of the "efi-build"
 * task, see `src-tauri/src/build/progress.rs`): the phase, its position, and
 * while a phase works through a list the current item with byte counts.
 */

import type { MessageKey } from '../i18n';

/** Build phases in the order the backend runs them. */
export const BUILD_PHASES = [
  'plan',
  'opencore',
  'resources',
  'assemble',
  'kexts',
  'acpi',
  'kernel-patches',
  'config',
  'save',
  'validate',
] as const;

export type BuildPhase = (typeof BUILD_PHASES)[number];

/** From the save phase on the build exists; the backend refuses to cancel it. */
export const UNCANCELLABLE_PHASES: readonly BuildPhase[] = ['save', 'validate'];

export interface BuildDetail {
  phase: BuildPhase;
  /** 1-based position of `phase`. */
  step: number;
  total: number;
  item: string | null;
  /** 1-based position of `item` within the phase. */
  index: number | null;
  count: number | null;
  downloaded: number | null;
  size: number | null;
}

/** One item a phase worked on (a kext package, an ACPI table, a download). */
export interface BuildItem {
  name: string;
  index: number;
  count: number;
  downloaded: number | null;
  size: number | null;
}

/** Items seen per phase, in the order they appeared. */
export type BuildItems = Partial<Record<BuildPhase, BuildItem[]>>;

export type PhaseState = 'done' | 'active' | 'pending' | 'skipped' | 'failed' | 'cancelled';

/** Message key of a build phase. */
export function buildPhaseLabel(phase: BuildPhase): MessageKey {
  return `build.phase.${phase}`;
}

export function isBuildPhase(value: unknown): value is BuildPhase {
  return typeof value === 'string' && (BUILD_PHASES as readonly string[]).includes(value);
}

function count(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) && value >= 0 ? Math.floor(value) : null;
}

/** Validate the `detail` of an efi-build task update; anything malformed is null. */
export function parseBuildDetail(raw: unknown): BuildDetail | null {
  if (typeof raw !== 'object' || raw === null || Array.isArray(raw)) return null;
  const d = raw as Record<string, unknown>;
  if (!isBuildPhase(d.phase)) return null;
  const position = BUILD_PHASES.indexOf(d.phase) + 1;
  const item = typeof d.item === 'string' && d.item.trim() ? d.item.trim() : null;
  const index = count(d.index);
  const total = count(d.count);
  const downloaded = count(d.downloaded);
  const size = count(d.size);
  return {
    phase: d.phase,
    step: count(d.step) || position,
    total: count(d.total) || BUILD_PHASES.length,
    item,
    index: item && index ? index : null,
    count: item && total ? total : null,
    downloaded: item ? downloaded : null,
    size: item && size ? size : null,
  };
}

/** Record the item of `detail` (if any): a new item is appended, a known one gets its newest byte counts. */
export function recordItem(items: BuildItems, detail: BuildDetail): BuildItems {
  if (!detail.item) return items;
  const list = items[detail.phase] ?? [];
  const existing = list.findIndex((i) => i.name === detail.item);
  const previous = existing === -1 ? null : list[existing];
  const next: BuildItem = {
    name: detail.item,
    index: detail.index ?? previous?.index ?? list.length + 1,
    count: detail.count ?? previous?.count ?? 0,
    downloaded: detail.downloaded ?? previous?.downloaded ?? null,
    size: detail.size ?? previous?.size ?? null,
  };
  const updated = existing === -1 ? [...list, next] : list.map((i, n) => (n === existing ? next : i));
  return { ...items, [detail.phase]: updated };
}

/**
 * State of every phase. `seen` holds the phases reported so far; a phase the
 * build passed without reporting it (no AMD patches on Intel, no picker
 * resources) is "skipped".
 */
export function phaseStates(
  current: BuildPhase | null,
  seen: readonly BuildPhase[],
  outcome: 'running' | 'done' | 'failed' | 'cancelled',
): PhaseState[] {
  const at = current ? BUILD_PHASES.indexOf(current) : -1;
  return BUILD_PHASES.map((phase, i) => {
    if (outcome === 'done') return seen.includes(phase) || seen.length === 0 ? 'done' : 'skipped';
    if (i < at) return seen.includes(phase) ? 'done' : 'skipped';
    if (i === at) {
      if (outcome === 'failed') return 'failed';
      if (outcome === 'cancelled') return 'cancelled';
      return 'active';
    }
    return 'pending';
  });
}

/** State of an item of the phase that is running now (or already passed). */
export function itemDone(item: BuildItem, detail: BuildDetail | null, phase: BuildPhase): boolean {
  if (!detail) return false;
  const at = BUILD_PHASES.indexOf(detail.phase);
  const own = BUILD_PHASES.indexOf(phase);
  if (at > own) return true;
  if (at < own) return false;
  if (detail.index !== null && item.index < detail.index) return true;
  return item.size !== null && item.downloaded !== null && item.downloaded >= item.size && detail.item !== item.name;
}
