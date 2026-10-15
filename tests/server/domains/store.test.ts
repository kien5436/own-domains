import { mkdtemp, readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { describe, expect, it } from 'vitest';

import { SettingsStore } from '../../../src/server/domains/store';

describe('SettingsStore', () => {
  it('creates defaults and persists settings atomically', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'own-domains-store-'));
    const store = new SettingsStore(directory);

    const initial = await store.load();

    expect(initial).toMatchObject({ schemaVersion: 1, revision: 0, mappings: [] });

    const saved = await store.save({ ...initial, revision: 1, lanAddress: '192.168.1.20' });

    expect(saved.revision).toBe(1);
    expect(JSON.parse(await readFile(join(directory, 'settings.json'), 'utf8'))).toEqual(saved);
  });

  it('rejects corrupt settings instead of silently replacing them', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'own-domains-store-'));

    await writeFile(join(directory, 'settings.json'), '{broken');

    await expect(new SettingsStore(directory).load()).rejects.toThrow(/corrupt/i);
  });

  it('rejects stale expected revisions', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'own-domains-store-'));
    const store = new SettingsStore(directory);
    const initial = await store.load();

    await store.save({ ...initial, revision: 1 }, 0);

    await expect(store.save({ ...initial, revision: 2 }, 0)).rejects.toThrow(/revision/i);
  });

  it('rejects persisted mappings that violate registry invariants', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'own-domains-store-'));

    await writeFile(join(directory, 'settings.json'), JSON.stringify({
      schemaVersion: 1,
      revision: 1,
      lanAddress: '192.168.1.20',
      upstreamDns: ['1.1.1.1'],
      mappings: [
        { id: 'same', hostname: 'shop.own', upstreamPort: 3000, tls: false, enabled: true },
        { id: 'same', hostname: 'SHOP.OWN.', upstreamPort: 3001, tls: false, enabled: true },
      ],
    }));

    await expect(new SettingsStore(directory).load()).rejects.toThrow(/corrupt/i);
  });
});
