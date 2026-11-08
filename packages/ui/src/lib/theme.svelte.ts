// Light / dark theme. The user picks `light`, `dark` or `system` (the default, which
// follows `prefers-color-scheme`); the choice is kept in localStorage and the resolved
// theme is written to `<html data-theme>`, which the tokens in `styles.css` read.
// Apps set the attribute before the first paint with the same rules (see the web
// app's `app.html`), so there is no flash of the wrong theme.

export const THEME_MODES = ['system', 'light', 'dark'] as const;
export type ThemeMode = (typeof THEME_MODES)[number];
export type Theme = 'light' | 'dark';

/** localStorage key of the user's choice (also read by `app.html`). */
export const THEME_STORAGE_KEY = 'cloudit.theme';

const DARK_QUERY = '(prefers-color-scheme: dark)';

let mode = $state<ThemeMode>('system');
let systemDark = $state(false);

export function isThemeMode(value: unknown): value is ThemeMode {
  return typeof value === 'string' && (THEME_MODES as readonly string[]).includes(value);
}

/** The theme to show for a choice and the system preference. */
export function resolveTheme(choice: ThemeMode, prefersDark: boolean): Theme {
  if (choice === 'system') return prefersDark ? 'dark' : 'light';
  return choice;
}

/** The user's choice. */
export function getThemeMode(): ThemeMode {
  return mode;
}

/** The theme in effect. */
export function getTheme(): Theme {
  return resolveTheme(mode, systemDark);
}

function apply(): void {
  if (typeof document !== 'undefined') document.documentElement.dataset.theme = getTheme();
}

function readStored(): string | null {
  try {
    return localStorage.getItem(THEME_STORAGE_KEY);
  } catch {
    return null; // storage disabled: follow the system
  }
}

let initialized = false;

/** Loads the stored choice, applies it and follows system changes. Idempotent. */
export function initTheme(): void {
  if (initialized || typeof window === 'undefined') return;
  initialized = true;
  const stored = readStored();
  mode = isThemeMode(stored) ? stored : 'system';
  const media = window.matchMedia(DARK_QUERY);
  systemDark = media.matches;
  media.addEventListener('change', (event) => {
    systemDark = event.matches;
    apply();
  });
  apply();
}

/** Changes and remembers the choice. */
export function setThemeMode(next: ThemeMode): void {
  mode = next;
  try {
    if (next === 'system') localStorage.removeItem(THEME_STORAGE_KEY);
    else localStorage.setItem(THEME_STORAGE_KEY, next);
  } catch {
    // Not persisted; applies to this session.
  }
  apply();
}
