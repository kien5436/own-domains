import { chmod, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';

import type { SetupPlan } from '../../src/cli/setup-plan';
import { inspectSetupPlan, parsePortListeners, type PortListener } from '../../src/cli/setup-preflight';

const temporaryDirectories: string[] = [];

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map((directory) => rm(directory, { recursive: true, force: true })));
});

describe('inspectSetupPlan', () => {
  it('blocks a differing file that is not owned by an existing manifest', async () => {
    const directory = await temporaryDirectory();
    const target = join(directory, 'own');

    await writeFile(target, 'unrelated configuration\n');
    const issues = await inspectSetupPlan(planFor(target), options());

    expect(issues).toContainEqual(expect.objectContaining({ severity: 'error', message: expect.stringMatching(/conflict.*own/i) }));
  });

  it('blocks an artifact with unexpected ownership', async () => {
    const directory = await temporaryDirectory();
    const target = join(directory, 'own');

    await writeFile(target, 'nameserver 127.0.0.1\n');
    await chmod(target, 0o644);
    const issues = await inspectSetupPlan(planFor(target), options({ userId: (process.getuid?.() ?? 0) + 1 }));

    expect(issues).toContainEqual(expect.objectContaining({ severity: 'error', message: expect.stringMatching(/owner.*own/i) }));
  });

  it('blocks unrelated listeners on required addresses but permits WARP on its own loopback', async () => {
    const listeners: PortListener[] = [
      { address: '127.0.2.2', port: 53, process: 'CloudflareWARP' },
      { address: '*', port: 80, process: 'nginx' },
      { address: '192.168.1.20', port: 443, process: 'caddy' },
    ];
    const issues = await inspectSetupPlan(
      { artifacts: [] },
      options({ inspectListeners: () => Promise.resolve(listeners), listenAddresses: ['127.0.0.1', '192.168.1.20'] }),
    );

    expect(issues).toContainEqual(expect.objectContaining({ severity: 'error', message: expect.stringMatching(/nginx.*80/i) }));
    expect(issues.some((issue) => /CloudflareWARP|443/.test(issue.message))).toBe(false);
  });
});

describe('parsePortListeners', () => {
  it('extracts process, address, and required port from lsof output', () => {
    const output = 'COMMAND PID USER FD TYPE DEVICE SIZE/OFF NODE NAME\nnginx 20 root 6u IPv4 0x1 0t0 TCP *:80 (LISTEN)\nCloudflareWARP 30 root 8u IPv4 0x2 0t0 UDP 127.0.2.2:53\n';

    expect(parsePortListeners(output)).toEqual([
      { address: '*', port: 80, process: 'nginx' },
      { address: '127.0.2.2', port: 53, process: 'CloudflareWARP' },
    ]);
  });
});

function planFor(path: string): SetupPlan {
  return { artifacts: [{ path, content: 'nameserver 127.0.0.1\n', mode: 0o644, backupExisting: true }] };
}

function options(overrides: Partial<Parameters<typeof inspectSetupPlan>[1]> = {}): NonNullable<Parameters<typeof inspectSetupPlan>[1]> {
  return {
    inspectListeners: () => Promise.resolve([]),
    listenAddresses: ['127.0.0.1'],
    userId: process.getuid?.() ?? 0,
    ...overrides,
  };
}

async function temporaryDirectory(): Promise<string> {
  const directory = await mkdtemp(join(tmpdir(), 'setup-preflight-'));

  temporaryDirectories.push(directory);

  return directory;
}
