export type Lang = 'en' | 'tr';

export const LANGUAGES: readonly { id: Lang; label: string }[] = [
  { id: 'en', label: 'English' },
  { id: 'tr', label: 'Türkçe' },
];

/** Content written once per language (troubleshooting entries, guides). */
export type Localized<T> = Record<Lang, T>;

export const STORAGE_KEY = 'oneclick.language';

/** Pick the UI language from the browser preference list; English is the fallback. */
export function detectLanguage(preferred: string | readonly string[] | null | undefined): Lang {
  const list = typeof preferred === 'string' ? [preferred] : preferred ?? [];
  for (const tag of list) {
    const base = tag.toLowerCase().split(/[-_]/)[0];
    if (base === 'tr') return 'tr';
    if (base === 'en') return 'en';
  }
  return 'en';
}

export function isLang(value: unknown): value is Lang {
  return value === 'en' || value === 'tr';
}
