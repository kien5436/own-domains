import { chmod, mkdir, mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, join } from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';

import type { SetupPlan } from '../../src/cli/setup-plan';
import { applySetupPlan } from '../../src/cli/setup-writer';

const temporaryDirectories: string[] = [];

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map((directory) => rm(directory, { recursive: true, force: true })));
});

describe('applySetupPlan', () => {
  it('atomically writes artifacts and backs up existing files', async () => {
    const directory = await temporaryDirectory();
    const target = join(directory, 'etc/resolver/own');
    const backupDirectory = join(directory, 'backups');

    await mkdir(join(directory, 'etc/resolver'), { recursive: true });
    await writeFile(target, 'old resolver\n');
    await chmod(target, 0o600);
    const result = await applySetupPlan(planFor(target, 'nameserver 127.0.0.1\n'), backupDirectory);

    expect(await readFile(target, 'utf8')).toBe('nameserver 127.0.0.1\n');
    expect((await stat(target)).mode & 0o777).toBe(0o644);
    expect(await readFile(result.backups[0]!, 'utf8')).toBe('old resolver\n');
  });

  it('skips an artifact that already has the requested content and mode', async () => {
    const directory = await temporaryDirectory();
    const target = join(directory, 'own.conf');
    const plan = planFor(target, 'current\n');

    await writeFile(target, 'current\n', { mode: 0o644 });
    const result = await applySetupPlan(plan, join(directory, 'backups'));

    expect(result).toEqual({ written: [], skipped: [target], backups: [] });
  });

  it('restores earlier artifacts when a later write fails', async () => {
    const directory = await temporaryDirectory();
    const first = join(directory, 'first.conf');
    const blockingFile = join(directory, 'blocked');
    const impossible = join(blockingFile, 'second.conf');

    await writeFile(first, 'before\n');
    await writeFile(blockingFile, 'not a directory');
    const plan: SetupPlan = { artifacts: [artifact(first, 'after\n'), artifact(impossible, 'never\n')] };

    await expect(applySetupPlan(plan, join(directory, 'backups'))).rejects.toThrow();
    expect(await readFile(first, 'utf8')).toBe('before\n');
  });

  it('applies an explicit owner to the artifact directory', async () => {
    const directory = await temporaryDirectory();
    const applicationDirectory = join(directory, 'Library/Application Support/OwnDomains');
    const target = join(applicationDirectory, 'settings-bootstrap.json');
    const ownerId = process.getuid?.();

    if (undefined === ownerId) throw new Error('test requires a POSIX user id');
    await applySetupPlan({ artifacts: [{ ...artifact(target, '{}\n'), ownerId, parentOwnerId: ownerId }] }, join(directory, 'backups'));

    expect((await stat(applicationDirectory)).uid).toBe(ownerId);
  });
});

function artifact(path: string, content: string) {
  return { path, content, mode: 0o644, backupExisting: true };
}

function planFor(path: string, content: string): SetupPlan {
  return { artifacts: [artifact(path, content)] };
}

async function temporaryDirectory(): Promise<string> {
  const directory = await mkdtemp(join(tmpdir(), `${basename(import.meta.filename)}-`));

  temporaryDirectories.push(directory);

  return directory;
}
