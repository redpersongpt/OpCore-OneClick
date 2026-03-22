export function normalizeErrorMessage(input: string | Error | null | undefined): string {
  let normalized = input instanceof Error ? input.message : String(input ?? '');
  let previous = '';

  while (normalized !== previous) {
    previous = normalized;
    normalized = normalized
      .replace(/^Error invoking remote method '[^']+':\s*/i, '')
      .replace(/^Error:\s*/i, '')
      .trim();
  }

  return normalized;
}

export function normalizeErrorMessageLower(input: string | Error | null | undefined): string {
  return normalizeErrorMessage(input).toLowerCase();
}
