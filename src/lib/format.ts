/** Clamp to 0..1; NaN, Infinity and non-numbers become null. */
export function toFraction(value: unknown): number | null {
  if (typeof value !== 'number' || !Number.isFinite(value)) return null;
  return Math.min(1, Math.max(0, value));
}

/** "42%" for a 0..1 fraction, or null when the value is unknown. Never "NaN%". */
export function formatPercent(fraction: unknown): string | null {
  const f = toFraction(fraction);
  return f === null ? null : `${Math.round(f * 100)}%`;
}

const UNITS = ['B', 'KB', 'MB', 'GB', 'TB'];

/** Decimal units, matching how drive vendors label capacity. */
export function formatBytes(bytes: number | null | undefined): string {
  if (typeof bytes !== 'number' || !Number.isFinite(bytes) || bytes < 0) return '?';
  let value = bytes;
  let unit = 0;
  while (value >= 1000 && unit < UNITS.length - 1) {
    value /= 1000;
    unit += 1;
  }
  const digits = unit === 0 || value >= 100 ? 0 : 1;
  return `${value.toFixed(digits)} ${UNITS[unit]}`;
}

/** Graphics memory is sold in binary units: 8192 MB → "8 GB". */
export function formatVram(megabytes: number | null | undefined): string | null {
  if (typeof megabytes !== 'number' || !Number.isFinite(megabytes) || megabytes <= 0) return null;
  if (megabytes < 1024) return `${Math.round(megabytes)} MB`;
  const gb = megabytes / 1024;
  return `${Number.isInteger(gb) ? gb : gb.toFixed(1)} GB`;
}

/** Keep the last `visible` characters, mask the rest. */
export function maskSecret(value: string, visible = 3): string {
  if (!value) return '';
  if (value.length <= visible) return '•'.repeat(value.length);
  return '•'.repeat(value.length - visible) + value.slice(-visible);
}

/** 0x10ec0897 → "0x10EC0897" */
export function formatCodecId(id: number | null | undefined): string {
  if (typeof id !== 'number' || !Number.isFinite(id)) return '';
  return `0x${(id >>> 0).toString(16).toUpperCase().padStart(8, '0')}`;
}

/** Accepts "0x10ec0897", "10EC0897", "10ec:0897" (vendor + device, 8 hex digits). */
export function parseCodecId(text: string): number | null {
  const cleaned = text.trim().replace(/^0x/i, '').replace(/[:\s-]/g, '');
  if (!/^[0-9a-f]{8}$/i.test(cleaned)) return null;
  return parseInt(cleaned, 16) >>> 0;
}

/** Lowercase 4-digit hex PCI id ("8086"), or null when invalid/empty. */
export function normalizePciId(text: string): string | null {
  const cleaned = text.trim().replace(/^0x/i, '').toLowerCase();
  return /^[0-9a-f]{4}$/.test(cleaned) ? cleaned : null;
}

/** Largest value of a Rust `u32` field (cores, RAM, layout-id, timeout). */
export const U32_MAX = 4_294_967_295;

/** Parse a non-negative integer field; empty, invalid or out-of-range (> u32) text gives null. */
export function parseCount(text: string): number | null {
  const trimmed = text.trim();
  if (!/^\d+$/.test(trimmed)) return null;
  const n = Number(trimmed);
  return Number.isSafeInteger(n) && n <= U32_MAX ? n : null;
}

export function formatDateTime(epochSeconds: number | null | undefined, locale: string): string | null {
  if (typeof epochSeconds !== 'number' || !Number.isFinite(epochSeconds)) return null;
  try {
    return new Date(epochSeconds * 1000).toLocaleString(locale);
  } catch {
    return null;
  }
}
