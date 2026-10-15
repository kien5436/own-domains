import { createHash } from 'node:crypto';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';

import { applyUninstallPlan, buildUninstallPlan } from '../../src/cli/uninstall-plan';

const temporaryDirectories: string[] = [];

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map((directory) => rm(directory, { recursive: true, force: true })));
});

describe('buildUninstallPlan', () => {
  it('restores backups, removes owned files, and preserves modified files', async () => {
    const fixture = await createFixture();
    const plan = await buildUninstallPlan(fixture.manifestPath);

    expect(plan.actions).toEqual([
      { kind: 'restore', path: fixture.restorePath, backupPath: fixture.restoreBackup },
      { kind: 'remove', path: fixture.removePath },
      { kind: 'preserve', path: fixture.modifiedPath, reason: 'content changed after setup' },
    ]);
    expect(plan.actions.some((action) => action.path === fixture.caPath)).toBe(false);
  });
});

describe('applyUninstallPlan', () => {
  it('executes only restore and remove actions and leaves modified files untouched', async () => {
    const fixture = await createFixture();
    const plan = await buildUninstallPlan(fixture.manifestPath);

    await applyUninstallPlan(plan);

    expect(await readFile(fixture.restorePath, 'utf8')).toBe('previous\n');
    await expect(readFile(fixture.removePath, 'utf8')).rejects.toMatchObject({ code: 'ENOENT' });
    expect(await readFile(fixture.modifiedPath, 'utf8')).toBe('user edit\n');
    expect(await readFile(fixture.caPath, 'utf8')).toBe('public certificate\n');
  });
});

async function createFixture() {
  const directory = await temporaryDirectory();
  const backupDirectory = join(directory, 'backups');
  const restorePath = join(directory, 'etc/resolver/own');
  const removePath = join(directory, 'Library/LaunchDaemons/dev.owndomains.caddy.plist');
  const modifiedPath = join(directory, 'Library/LaunchAgents/dev.owndomains.backend.plist');
  const restoreBackup = join(backupDirectory, restorePath.replace(/^\/+/, ''));
  const caPath = join(directory, 'Library/Application Support/Caddy/root.crt');
  const installed = 'installed\n';

  await Promise.all([restorePath, removePath, modifiedPath, restoreBackup, caPath].map((path) => mkdir(join(path, '..'), { recursive: true })));
  await Promise.all([
    writeFile(restorePath, installed),
    writeFile(removePath, installed),
    writeFile(modifiedPath, 'user edit\n'),
    writeFile(restoreBackup, 'previous\n'),
    writeFile(caPath, 'public certificate\n'),
  ]);
  const manifestPath = join(directory, 'install-manifest.json');
  const manifest = {
    schemaVersion: 1,
    backupDirectory,
    artifacts: [restorePath, removePath, modifiedPath].map((path) => ({ path, sha256: sha256(installed) })),
  };

  await writeFile(manifestPath, `${JSON.stringify(manifest)}\n`);

  return { caPath, manifestPath, modifiedPath, removePath, restoreBackup, restorePath };
}

function sha256(value: string): string {
  return createHash('sha256').update(value).digest('hex');
}

async function temporaryDirectory(): Promise<string> {
  const directory = await mkdtemp(join(tmpdir(), 'uninstall-plan-'));

  temporaryDirectories.push(directory);

  return directory;
}
