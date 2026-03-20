const HOME_PATH_PATTERNS: Array<{ pattern: RegExp; replacement: (match: string) => string }> = [
  {
    pattern: /\/Users\/[^/\s"'`,\]]+(?:\/[^\s"'`,\]]+)*/g,
    replacement: (match) => summarizeAbsolutePath(match) ?? '[path:redacted]',
  },
  {
    pattern: /\/home\/[^/\s"'`,\]]+(?:\/[^\s"'`,\]]+)*/g,
    replacement: (match) => summarizeAbsolutePath(match) ?? '[path:redacted]',
  },
  {
    pattern: /[A-Za-z]:[/\\]Users[/\\][^/\\\s"'`,\]]+(?:[/\\][^/\\\s"'`,\]]+)*/g,
    replacement: (match) => summarizeAbsolutePath(match) ?? '[path:redacted]',
  },
  {
    pattern: /\/private\/var\/folders\/[^\s"'`,\]]+/g,
    replacement: () => '[path:private-temp]',
  },
  {
    pattern: /\/var\/folders\/[^\s"'`,\]]+/g,
    replacement: () => '[path:temp]',
  },
];

const EMAIL_PATTERN = /\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b/gi;
const FLASH_TOKEN_PATTERN = /\bflashconf\.[A-Za-z0-9._-]+\b/g;
const KEY_VALUE_SECRET_PATTERN = /\b(token|secret|signature|authorization|cookie|password|api[_-]?key)\b(\s*[:=]\s*)([^\s,;]+)/gi;
const MAC_DISK_PATTERN = /\/dev\/(disk\d+(?:s\d+)?)/gi;
const LINUX_DISK_PATTERN = /\/dev\/([a-z]+[a-z0-9]*)/gi;
const WINDOWS_DISK_PATTERN = /(?:\\\\\.\\)?(PHYSICALDRIVE\d+)/gi;

const SECRET_KEY_PATTERN = /(token|secret|signature|authorization|cookie|password|api[_-]?key)/i;
const SERIAL_KEY_PATTERN = /(serial(number)?|uuid|machine.?id|hardware.?id)/i;
const PATH_KEY_PATTERN = /(path|dest|destination|folder|directory|file|workspace|location)$/i;
const DEVICE_KEY_PATTERN = /(device|disk|mount(point)?|target)$/i;

function lastPathSegment(candidate: string): string | null {
  const segments = candidate.split(/[\\/]+/).filter(Boolean);
  return segments.length > 0 ? segments[segments.length - 1] : null;
}

export function summarizeAbsolutePath(candidate: string | null | undefined): string | null {
  if (!candidate) return null;
  const normalized = candidate.trim();
  if (!normalized) return null;

  const isAbsolutePath =
    normalized.startsWith('/') ||
    normalized.startsWith('~') ||
    /^[A-Za-z]:[\\/]/.test(normalized) ||
    normalized.startsWith('\\\\');

  if (!isAbsolutePath) return normalized;

  const basename = lastPathSegment(normalized);
  return basename ? `[path:${basename}]` : '[path:redacted]';
}

export function summarizeDeviceIdentifier(candidate: string | null | undefined): string | null {
  if (!candidate) return null;
  const normalized = candidate.trim();
  if (!normalized) return null;

  const macMatch = normalized.match(/\/dev\/(disk\d+(?:s\d+)?)/i);
  if (macMatch?.[1]) return macMatch[1];

  const linuxMatch = normalized.match(/\/dev\/([a-z]+[a-z0-9]*)/i);
  if (linuxMatch?.[1]) return linuxMatch[1];

  const windowsMatch = normalized.match(/(?:\\\\\.\\)?(PHYSICALDRIVE\d+)/i);
  if (windowsMatch?.[1]) return windowsMatch[1].toUpperCase();

  return redactSensitiveText(normalized);
}

export function redactSensitiveText(text: string | null | undefined): string {
  if (!text) return '';

  let redacted = String(text);

  for (const { pattern, replacement } of HOME_PATH_PATTERNS) {
    redacted = redacted.replace(pattern, replacement);
  }

  redacted = redacted
    .replace(EMAIL_PATTERN, '[redacted-email]')
    .replace(FLASH_TOKEN_PATTERN, '[redacted-token]')
    .replace(KEY_VALUE_SECRET_PATTERN, (_match, key: string, separator: string) => `${key}${separator}[redacted]`)
    .replace(MAC_DISK_PATTERN, (_match, disk: string) => disk)
    .replace(LINUX_DISK_PATTERN, (_match, disk: string) => disk)
    .replace(WINDOWS_DISK_PATTERN, (_match, disk: string) => disk.toUpperCase());

  return redacted;
}

export function sanitizeTelemetryValue(value: unknown, key?: string): unknown {
  if (value === null || value === undefined) return value;
  if (typeof value === 'number' || typeof value === 'boolean') return value;
  if (typeof value === 'string') {
    if (SECRET_KEY_PATTERN.test(key ?? '')) return '[redacted-token]';
    if (SERIAL_KEY_PATTERN.test(key ?? '')) return '[redacted-identifier]';
    if (PATH_KEY_PATTERN.test(key ?? '')) return summarizeAbsolutePath(value) ?? '[path:redacted]';
    if (DEVICE_KEY_PATTERN.test(key ?? '')) return summarizeDeviceIdentifier(value) ?? '[device:redacted]';
    return redactSensitiveText(value);
  }

  if (Array.isArray(value)) {
    return value.map((item) => sanitizeTelemetryValue(item, key));
  }

  if (typeof value === 'object') {
    return Object.fromEntries(
      Object.entries(value).map(([childKey, childValue]) => [childKey, sanitizeTelemetryValue(childValue, childKey)]),
    );
  }

  return value;
}
