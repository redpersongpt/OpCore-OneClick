/**
 * ts-rs declares Rust u64/i64 fields as `bigint`, but they cross the IPC
 * boundary as plain JSON numbers (and a real BigInt cannot be serialised by
 * `JSON.stringify`). Read them with `toNum` and create them with `asU64`.
 */
export function toNum(value: bigint | number | null | undefined): number | null {
  if (value === null || value === undefined) return null;
  const n = Number(value);
  return Number.isFinite(n) ? n : null;
}

export function asU64(value: number): bigint {
  return value as unknown as bigint;
}
