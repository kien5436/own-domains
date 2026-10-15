import { describe, expect, it } from 'vitest';

import { commandExists } from '../../src/cli/commands';

describe('commandExists', () => {
  it('finds an executable without invoking a shell', () => {
    expect(commandExists('node')).toBe(true);
  });

  it('returns false for a missing executable', () => {
    expect(commandExists('own-domains-command-that-does-not-exist')).toBe(false);
  });
});
