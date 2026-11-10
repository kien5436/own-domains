// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { applyTheme, getStoredTheme, resolveTheme, setStoredTheme, THEME_STORAGE_KEY } from '../../src/web/features/theme/theme';

function mockMatchMedia(matches: boolean) {
  window.matchMedia = vi.fn().mockReturnValue({ matches, addEventListener: vi.fn(), removeEventListener: vi.fn() }) as unknown as typeof window.matchMedia;
}

beforeEach(() => {
  window.localStorage.clear();
  document.documentElement.classList.remove('dark');
  mockMatchMedia(false);
});

describe('theme preference', () => {
  it('defaults to system when nothing is stored', () => {
    expect(getStoredTheme()).toBe('system');
  });

  it('falls back to system for invalid stored values', () => {
    window.localStorage.setItem(THEME_STORAGE_KEY, 'midnight');

    expect(getStoredTheme()).toBe('system');
  });

  it('returns the stored light or dark preference', () => {
    setStoredTheme('dark');

    expect(getStoredTheme()).toBe('dark');
  });

  it('resolves system to the OS color scheme', () => {
    mockMatchMedia(true);

    expect(resolveTheme('system')).toBe('dark');
    mockMatchMedia(false);

    expect(resolveTheme('system')).toBe('light');
  });

  it('applies the resolved theme to the document root', () => {
    applyTheme('dark');

    expect(document.documentElement.classList.contains('dark')).toBe(true);
    applyTheme('light');

    expect(document.documentElement.classList.contains('dark')).toBe(false);
  });
});
