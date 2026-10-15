import { createHash } from 'node:crypto';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';

import { executeUninstall, parseUninstallArguments } from '../../src/cli/uninstall-command';

const temporaryDirectories: string[] = [];

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map((directory) => rm(directory, { recursive: true, force: true })));
});

describe('executeUninstall', () => {
  it('previews manifest-owned actions without changing files', async () => {
    const { manifestPath, target } = await fixture();

    const result = await executeUninstall({ apply: false, manifestPath });

    expect(result).toMatchObject({ exitCode: 0, stdout: expect.stringContaining(`REMOVE ${target}`) });
    expect(await readFile(target, 'utf8')).toBe('installed\n');
  });

  it('applies the same manifest-owned action plan explicitly', async () => {
    const { manifestPath, target } = await fixture();

    const result = await executeUninstall({ apply: true, manifestPath });

    expect(result).toMatchObject({ exitCode: 0, stdout: expect.stringContaining(`REMOVED ${target}`) });
    await expect(readFile(target, 'utf8')).rejects.toMatchObject({ code: 'ENOENT' });
  });
});

describe('parseUninstallArguments', () => {
  it('keeps apply opt-in and accepts pnpm separators', () => {
    expect(parseUninstallArguments(['--', '--dry-run'])).toEqual({ apply: false });
    expect(parseUninstallArguments(['--apply'])).toEqual({ apply: true });
    expect(() => parseUninstallArguments(['--force'])).toThrow(/Unknown argument/);
  });
});

async function fixture() {
  const directory = await mkdtemp(join(tmpdir(), 'uninstall-command-'));
  const target = join(directory, 'owned.conf');
  const content = 'installed\n';

  temporaryDirectories.push(directory);
  await mkdir(directory, { recursive: true });
  await writeFile(target, content);
  const manifestPath = join(directory, 'install-manifest.json');

  await writeFile(manifestPath, `${JSON.stringify({ schemaVersion: 1, backupDirectory: join(directory, 'backups'), artifacts: [{ path: target, sha256: createHash('sha256').update(content).digest('hex') }] })}\n`);

  return { manifestPath, target };
}
