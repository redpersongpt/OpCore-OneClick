import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from 'react';
import { en, type MessageKey } from './en';
import { detectLanguage, isLang, STORAGE_KEY, type Lang } from './lang';
import { tr } from './tr';

export type { MessageKey } from './en';
export { LANGUAGES, detectLanguage, type Lang, type Localized } from './lang';

export type Params = Record<string, string | number>;
export type Translate = (key: MessageKey, params?: Params) => string;

export const DICTIONARIES: Record<Lang, Record<MessageKey, string>> = { en, tr };

export function interpolate(template: string, params?: Params): string {
  if (!params) return template;
  return template.replace(/\{(\w+)\}/g, (match, name: string) =>
    Object.prototype.hasOwnProperty.call(params, name) ? String(params[name]) : match,
  );
}

export function translate(lang: Lang, key: MessageKey, params?: Params): string {
  const template = DICTIONARIES[lang][key] ?? en[key] ?? key;
  return interpolate(template, params);
}

function initialLanguage(): Lang {
  try {
    const stored = window.localStorage.getItem(STORAGE_KEY);
    if (isLang(stored)) return stored;
  } catch {
    // Storage unavailable.
  }
  if (typeof navigator === 'undefined') return 'en';
  return detectLanguage(navigator.languages?.length ? navigator.languages : navigator.language);
}

interface I18nValue {
  lang: Lang;
  setLang: (lang: Lang) => void;
  t: Translate;
  /** BCP 47 locale for number/date formatting. */
  locale: string;
}

const I18nContext = createContext<I18nValue>({
  lang: 'en',
  setLang: () => undefined,
  t: (key, params) => translate('en', key, params),
  locale: 'en-US',
});

export function I18nProvider({ children, initial }: { children: ReactNode; initial?: Lang }) {
  const [lang, setLangState] = useState<Lang>(() => initial ?? initialLanguage());

  const setLang = useCallback((next: Lang) => {
    setLangState(next);
    try {
      window.localStorage.setItem(STORAGE_KEY, next);
    } catch {
      // Not persisted; the choice still applies to this session.
    }
  }, []);

  useEffect(() => {
    document.documentElement.lang = lang;
  }, [lang]);

  const value = useMemo<I18nValue>(
    () => ({
      lang,
      setLang,
      t: (key, params) => translate(lang, key, params),
      locale: lang === 'tr' ? 'tr-TR' : 'en-US',
    }),
    [lang, setLang],
  );

  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

export function useI18n(): I18nValue {
  return useContext(I18nContext);
}

export function useT(): Translate {
  return useContext(I18nContext).t;
}
