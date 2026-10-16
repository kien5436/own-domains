import { createHash } from 'node:crypto';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';

import type { SetupPlan } from '../../src/cli/setup-plan';
import { executeSetup, parseSetupArguments } from '../../src/cli/setup-command';

const temporaryDirectories: string[] = [];

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map((directory) => rm(directory, { recursive: true, force: true })));
});

describe('executeSetup', () => {
  it('previews every artifact without writing in dry-run mode', async () => {
    const directory = await temporaryDirectory();
    const target = join(directory, 'resolver/own');

    const result = await executeSetup({ apply: false, backupDirectory: join(directory, 'backups'), plan: planFor(target) });

    expect(result.exitCode).toBe(0);
    expect(result.stdout).toContain(`CREATE ${target}`);
    await expect(readFile(target, 'utf8')).rejects.toMatchObject({ code: 'ENOENT' });
  });

  it('blocks apply when preflight finds an error', async () => {
    const directory = await temporaryDirectory();
    const target = join(directory, 'resolver/own');

    const result = await executeSetup(
      { apply: true, backupDirectory: join(directory, 'backups'), plan: planFor(target) },
      { inspect: () => Promise.resolve([{ severity: 'error', message: 'port 53 is already in use' }]) },
    );

    expect(result).toMatchObject({ exitCode: 2, stderr: expect.stringContaining('port 53 is already in use') });
    await expect(readFile(target, 'utf8')).rejects.toMatchObject({ code: 'ENOENT' });
  });

  it('applies the plan after a clean preflight', async () => {
    const directory = await temporaryDirectory();
    const target = join(directory, 'resolver/own');

    const result = await executeSetup(
      { apply: true, backupDirectory: join(directory, 'backups'), plan: planFor(target) },
      { inspect: () => Promise.resolve([]) },
    );

    expect(result).toMatchObject({ exitCode: 0, stdout: expect.stringContaining(`WROTE ${target}`), stderr: '' });
    expect(await readFile(target, 'utf8')).toBe('nameserver 127.0.0.1\n');
  });

  it('rolls back installed artifacts when service activation fails', async () => {
    const directory = await temporaryDirectory();
    const target = join(directory, 'resolver/own');
    const backupDirectory = join(directory, 'backups');
    const content = 'nameserver 127.0.0.1\n';
    const unchanged = join(directory, 'unchanged.conf');
    const unchangedContent = 'keep me\n';
    const manifestPath = join(directory, 'install-manifest.json');
    const manifest = { schemaVersion: 1, backupDirectory, artifacts: [{ path: target, sha256: createHash('sha256').update(content).digest('hex') }, { path: unchanged, sha256: createHash('sha256').update(unchangedContent).digest('hex') }] };
    const plan: SetupPlan = {
      artifacts: [
        { path: target, content, mode: 0o644, backupExisting: true },
        { path: unchanged, content: unchangedContent, mode: 0o644, backupExisting: true },
        { path: manifestPath, content: `${JSON.stringify(manifest)}\n`, mode: 0o600, backupExisting: false },
      ],
    };

    await writeFile(unchanged, unchangedContent, { mode: 0o644 });

    const lifecycle: string[] = [];
    const result = await executeSetup(
      { apply: true, backupDirectory, plan },
      {
        activate() {
          lifecycle.push('activate');

          return Promise.reject(new Error('launchctl failed'));
        },
        deactivate() {
          lifecycle.push('deactivate');

          return Promise.resolve();
        },
        inspect: () => Promise.resolve([]),
      },
    );

    expect(result).toMatchObject({ exitCode: 2, stderr: expect.stringContaining('launchctl failed') });
    expect(lifecycle).toEqual(['activate', 'deactivate']);
    await expect(readFile(target, 'utf8')).rejects.toMatchObject({ code: 'ENOENT' });
    expect(await readFile(unchanged, 'utf8')).toBe(unchangedContent);
    await expect(readFile(manifestPath, 'utf8')).rejects.toMatchObject({ code: 'ENOENT' });
  });
});

describe('parseSetupArguments', () => {
  it('requires an explicit LAN address and keeps apply opt-in', () => {
    expect(parseSetupArguments(['--lan-address', '192.168.1.20'])).toEqual({ apply: false, lanAddress: '192.168.1.20', upstreamDns: ['1.1.1.1', '9.9.9.9'] });
    expect(parseSetupArguments(['--', '--dry-run', '--lan-address', '192.168.1.20'])).toEqual({ apply: false, lanAddress: '192.168.1.20', upstreamDns: ['1.1.1.1', '9.9.9.9'] });
    expect(parseSetupArguments(['--apply', '--lan-address', '192.168.1.20', '--upstream-dns', '8.8.8.8,1.1.1.1'])).toEqual({ apply: true, lanAddress: '192.168.1.20', upstreamDns: ['8.8.8.8', '1.1.1.1'] });
  });

  it('rejects missing values and unknown arguments before planning', () => {
    expect(() => parseSetupArguments([])).toThrow(/--lan-address/);
    expect(() => parseSetupArguments(['--lan-address', '192.168.1.20', '--force'])).toThrow(/Unknown argument/);
  });
});

function planFor(path: string): SetupPlan {
  return { artifacts: [{ path, content: 'nameserver 127.0.0.1\n', mode: 0o644, backupExisting: true }] };
}

async function temporaryDirectory(): Promise<string> {
  const directory = await mkdtemp(join(tmpdir(), 'setup-command-'));

  temporaryDirectories.push(directory);

  return directory;
}
