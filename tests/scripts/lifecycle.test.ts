import { execFile } from 'node:child_process';
import { promisify } from 'node:util';

import { describe, expect, it } from 'vitest';

const execute = promisify(execFile);

describe('lifecycle scripts', () => {
  it('previews installation for a requested LAN address without changing the host', async () => {
    const result = await execute('./scripts/install.sh', ['--dry-run', '192.168.1.20']);

    expect(result.stdout).toContain('INSTALL locked dependencies');
    expect(result.stdout).toContain('BUILD dashboard and server');
    expect(result.stdout).toContain('SETUP 192.168.1.20');
  });

  it('requires a LAN address for installation', async () => {
    await expect(execute('./scripts/install.sh', [])).rejects.toMatchObject({ code: 2 });
  });

  it('previews service removal and an optional exact application-data purge', async () => {
    const result = await execute('./scripts/uninstall.sh', ['--dry-run', '--purge'], { env: { ...process.env, HOME: '/Users/tester', USER: 'tester' } });

    expect(result.stdout).toContain('UNLOAD dev.owndomains.backend');
    expect(result.stdout).toContain('UNLOAD dev.owndomains.caddy');
    expect(result.stdout).toContain('UNLOAD dev.owndomains.dnsmasq');
    expect(result.stdout).toContain('UNINSTALL managed setup files');
    expect(result.stdout).toContain('PURGE /Users/tester/Library/Application Support/OwnDomains');
  });
});
