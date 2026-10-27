import { describe, expect, it } from 'vitest';

import { stagedDaemonName } from '../../scripts/stage-desktop';

describe('desktop staging names', () => {
  it('adds .exe for windows targets and leaves others bare', () => {
    expect(stagedDaemonName('x86_64-pc-windows-msvc')).toBe('own-domains-daemon-x86_64-pc-windows-msvc.exe');
    expect(stagedDaemonName('aarch64-pc-windows-msvc')).toBe('own-domains-daemon-aarch64-pc-windows-msvc.exe');
    expect(stagedDaemonName('aarch64-apple-darwin')).toBe('own-domains-daemon-aarch64-apple-darwin');
    expect(stagedDaemonName('x86_64-apple-darwin')).toBe('own-domains-daemon-x86_64-apple-darwin');
  });
});
