// Reactive i18n: the current language is a `$state` rune, so any template or
// `$derived` that calls `t()` re-renders when the language changes.
import { en, type Dictionary, type MessageKey } from './en';
import { it } from './it';

export const LANGUAGES = ['en', 'it'] as const;
export type Lang = (typeof LANGUAGES)[number];
export const DEFAULT_LANG: Lang = 'en';

const dictionaries: Record<Lang, Dictionary> = { en, it };

let current = $state<Lang>(DEFAULT_LANG);

export function isLang(value: unknown): value is Lang {
  return typeof value === 'string' && (LANGUAGES as readonly string[]).includes(value);
}

/** The active language. */
export function getLang(): Lang {
  return current;
}

export function setLang(lang: Lang): void {
  current = lang;
}

/**
 * Picks the first supported language from a list of BCP 47 tags (e.g.
 * `navigator.languages`), falling back to `DEFAULT_LANG`.
 */
export function detectLang(preferred: readonly string[]): Lang {
  for (const tag of preferred) {
    const base = tag.toLowerCase().split('-')[0];
    if (isLang(base)) return base;
  }
  return DEFAULT_LANG;
}

/** Translates `key` in the active language, replacing `{name}` placeholders from `params`. */
export function t(key: MessageKey, params?: Record<string, string | number>): string {
  const template = dictionaries[current][key];
  if (!params) return template;
  return template.replace(/\{(\w+)\}/g, (match, name: string) =>
    name in params ? String(params[name]) : match
  );
}
