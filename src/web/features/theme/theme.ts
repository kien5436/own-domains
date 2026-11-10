export type ThemePreference = 'system' | 'light' | 'dark';

export type ResolvedTheme = 'light' | 'dark';

export const THEME_STORAGE_KEY = 'own-domains-theme';

const PREFERENCES: ReadonlySet<string> = new Set(['system', 'light', 'dark']);

export function getStoredTheme(): ThemePreference {
  try {
    const stored = window.localStorage.getItem(THEME_STORAGE_KEY);

    if (stored && PREFERENCES.has(stored)) return stored as ThemePreference;
  }
  catch {
    // Private mode or blocked storage: fall through to system.
  }

  return 'system';
}

export function setStoredTheme(preference: ThemePreference): void {
  try {
    window.localStorage.setItem(THEME_STORAGE_KEY, preference);
  }
  catch {
    // Non-fatal: the theme still applies for this session.
  }
}

export function resolveTheme(preference: ThemePreference): ResolvedTheme {
  if ('light' === preference) return 'light';

  if ('dark' === preference) return 'dark';

  return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
}

export function applyTheme(resolved: ResolvedTheme): void {
  document.documentElement.classList.toggle('dark', 'dark' === resolved);
}

export function currentPreference(): { preference: ThemePreference; resolved: ResolvedTheme } {
  const preference = getStoredTheme();
  const resolved = resolveTheme(preference);

  applyTheme(resolved);

  return { preference, resolved };
}
