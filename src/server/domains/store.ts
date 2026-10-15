import { mkdir, open, readFile, rename, writeFile } from 'node:fs/promises';
import { isIP } from 'node:net';
import { join } from 'node:path';

import type { Settings } from '../../shared/types';
import { validateMapping } from './validation';

const DEFAULT_SETTINGS: Settings = { schemaVersion: 1, revision: 0, lanAddress: '', upstreamDns: ['1.1.1.1', '9.9.9.9'], mappings: [] };

export class RevisionConflictError extends Error {}

export class CorruptSettingsError extends Error {}

export class SettingsStore {
  readonly #path: string;
  readonly #temporaryPath: string;

  constructor(private readonly directory: string) {
    this.#path = join(directory, 'settings.json');
    this.#temporaryPath = join(directory, 'settings.json.tmp');
  }

  async load(): Promise<Settings> {
    await mkdir(this.directory, { recursive: true, mode: 0o700 });

    try {
      const parsed = JSON.parse(await readFile(this.#path, 'utf8')) as Settings;

      return validateSettings(parsed);
    }
    catch (error) {
      if ('ENOENT' === (error as NodeJS.ErrnoException).code) return structuredClone(DEFAULT_SETTINGS);
      throw new CorruptSettingsError(`Settings are corrupt: ${String(error)}`);
    }
  }

  async save(settings: Settings, expectedRevision?: number): Promise<Settings> {
    const current = await this.load();

    if (expectedRevision !== undefined && current.revision !== expectedRevision) throw new RevisionConflictError('Settings revision is stale');
    await writeFile(this.#temporaryPath, `${JSON.stringify(settings, null, 2)}\n`, { mode: 0o600 });
    const handle = await open(this.#temporaryPath, 'r');

    await handle.sync();
    await handle.close();
    await rename(this.#temporaryPath, this.#path);

    return settings;
  }
}

function validateSettings(value: Settings): Settings {
  if (!value || 'object' !== typeof value || 1 !== value.schemaVersion || !Number.isInteger(value.revision) || 0 > value.revision || !Array.isArray(value.mappings) || !Array.isArray(value.upstreamDns)) throw new Error('unsupported settings schema');
  if (value.lanAddress && 4 !== isIP(value.lanAddress)) throw new Error('invalid LAN address');
  if (!value.upstreamDns.every((address) => 4 === isIP(address))) throw new Error('invalid upstream DNS');
  const mappings = value.mappings.map((mapping) => validateMapping(mapping));
  const ids = new Set(mappings.map((mapping) => mapping.id));
  const hostnames = new Set(mappings.map((mapping) => mapping.hostname));

  if (ids.size !== mappings.length || hostnames.size !== mappings.length) throw new Error('duplicate mapping identity');

  return { ...value, mappings };
}
