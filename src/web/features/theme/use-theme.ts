import { useCallback, useEffect, useState } from 'react';

import { applyTheme, getStoredTheme, type ResolvedTheme, setStoredTheme, type ThemePreference } from './theme';

function prefersDark(): boolean {
  if ('function' !== typeof window.matchMedia) return false;

  return window.matchMedia('(prefers-color-scheme: dark)').matches;
}

export function useTheme() {
  const [preference, setPreference] = useState<ThemePreference>(() => getStoredTheme());
  const [systemDark, setSystemDark] = useState<boolean>(() => prefersDark());
  const resolved: ResolvedTheme = 'system' === preference ? (systemDark ? 'dark' : 'light') : preference;

  useEffect(() => {
    applyTheme(resolved);
  }, [resolved]);

  useEffect(() => {
    if ('function' !== typeof window.matchMedia) return undefined;

    const query = window.matchMedia('(prefers-color-scheme: dark)');
    const onChange = (event: MediaQueryListEvent) => setSystemDark(event.matches);

    query.addEventListener('change', onChange);

    return () => query.removeEventListener('change', onChange);
  }, []);

  const choose = useCallback((next: ThemePreference) => {
    setStoredTheme(next);
    setPreference(next);
  }, []);

  return { preference, resolved, choose };
}
