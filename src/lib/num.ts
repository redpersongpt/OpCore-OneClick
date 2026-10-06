/**
 * Read a numeric IPC field defensively: JSON numbers can still arrive as null
 * or, from older saved state, as strings.
 */
export function toNum(value: number | string | null | undefined): number | null {
  if (value === null || value === undefined) return null;
  const n = Number(value);
  return Number.isFinite(n) ? n : null;
}
