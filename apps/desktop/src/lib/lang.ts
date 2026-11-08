// Picks and remembers the UI language (localStorage, else the system languages).
import { detectLang, isLang, setLang, type Lang } from '@cloudit/ui/i18n';

const STORAGE_KEY = 'cloudit.lang';

function stored(): string | null {
  try {
    return localStorage.getItem(STORAGE_KEY);
  } catch {
    return null;
  }
}

export function initLang(): void {
  const saved = stored();
  setLang(isLang(saved) ? saved : detectLang(navigator.languages));
}

export function changeLang(lang: Lang): void {
  setLang(lang);
  try {
    localStorage.setItem(STORAGE_KEY, lang);
  } catch {
    // Not persisted; the choice still applies to this session.
  }
}
