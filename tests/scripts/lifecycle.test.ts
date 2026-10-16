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

  it('reports a missing manifest while previewing every independent cleanup', async () => {
    const removal = execute('./scripts/uninstall.sh', ['--dry-run', '--purge'], { env: { ...process.env, HOME: '/Users/tester', USER: 'tester' } });

    await expect(removal).rejects.toMatchObject({
      code: 1,
      stderr: expect.stringMatching(/Cleanup finished with 1 error:\n- install manifest is missing; managed setup files will be skipped/),
      stdout: expect.stringMatching(/UNLOAD dev\.owndomains\.backend[\s\S]*UNLOAD dev\.owndomains\.caddy[\s\S]*UNLOAD dev\.owndomains\.dnsmasq[\s\S]*PURGE \/Users\/tester\/Library\/Application Support\/OwnDomains/),
    });
  });
});
