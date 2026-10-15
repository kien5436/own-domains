import { isIP } from 'node:net';

import type { ApplyResult, MappingInput, Settings } from '../../shared/types';
import { SettingsStore } from './store';
import { validateMapping, ValidationError } from './validation';

type Reconciler = (settings: Settings) => Promise<ApplyResult>;

export class DomainService {
  #queue: Promise<unknown> = Promise.resolve();

  constructor(private readonly store: SettingsStore, private readonly reconcile: Reconciler) {}

  create(input: MappingInput, expectedRevision: number): Promise<{ settings: Settings; apply: ApplyResult }> {
    return this.#serialize(async () => {
      const settings = await this.store.load();

      if (settings.revision !== expectedRevision) throw new Error('Settings revision is stale');
      const mapping = validateMapping({ ...input, id: undefined });

      if (settings.mappings.some((item) => item.hostname === mapping.hostname)) throw new Error(`${mapping.hostname} is already registered`);
      const updated = { ...settings, revision: settings.revision + 1, mappings: [...settings.mappings, mapping] } satisfies Settings;

      await this.store.save(updated, expectedRevision);

      return { settings: updated, apply: await this.reconcile(updated) };
    });
  }

  list(): Promise<Settings> {
    return this.store.load();
  }

  update(id: string, patch: Partial<Omit<MappingInput, 'id'>>, expectedRevision: number): Promise<{ settings: Settings; apply: ApplyResult }> {
    return this.#serialize(async () => {
      const settings = await this.store.load();

      if (settings.revision !== expectedRevision) throw new Error('Settings revision is stale');
      const index = settings.mappings.findIndex((mapping) => mapping.id === id);

      if (0 > index) throw new Error('Domain mapping not found');
      const mapping = validateMapping({ ...settings.mappings[index], ...patch, id });

      if (settings.mappings.some((item, itemIndex) => itemIndex !== index && item.hostname === mapping.hostname)) throw new Error(`${mapping.hostname} is already registered`);
      const mappings = [...settings.mappings];

      mappings[index] = mapping;
      const updated = { ...settings, revision: settings.revision + 1, mappings } satisfies Settings;

      await this.store.save(updated, expectedRevision);

      return { settings: updated, apply: await this.reconcile(updated) };
    });
  }

  remove(id: string, expectedRevision: number): Promise<{ settings: Settings; apply: ApplyResult }> {
    return this.#serialize(async () => {
      const settings = await this.store.load();

      if (settings.revision !== expectedRevision) throw new Error('Settings revision is stale');
      if (!settings.mappings.some((mapping) => mapping.id === id)) throw new Error('Domain mapping not found');
      const updated = { ...settings, revision: settings.revision + 1, mappings: settings.mappings.filter((mapping) => mapping.id !== id) } satisfies Settings;

      await this.store.save(updated, expectedRevision);

      return { settings: updated, apply: await this.reconcile(updated) };
    });
  }

  reconcileCurrent(): Promise<ApplyResult> {
    return this.#serialize(async () => this.reconcile(await this.store.load()));
  }

  updateNetwork(input: { lanAddress: string; upstreamDns: string[] }, expectedRevision: number): Promise<{ settings: Settings; apply: ApplyResult }> {
    return this.#serialize(async () => {
      if (4 !== isIP(input.lanAddress)) throw new ValidationError('LAN address must be an IPv4 address');
      if (!Array.isArray(input.upstreamDns) || !input.upstreamDns.length || !input.upstreamDns.every((address) => 4 === isIP(address))) throw new ValidationError('Upstream DNS must contain IPv4 addresses');
      if (input.upstreamDns.includes(input.lanAddress) || input.upstreamDns.includes('127.0.0.1')) throw new ValidationError('Upstream DNS would create a forwarding loop');
      const settings = await this.store.load();

      if (settings.revision !== expectedRevision) throw new Error('Settings revision is stale');
      const updated = { ...settings, revision: settings.revision + 1, lanAddress: input.lanAddress, upstreamDns: [...input.upstreamDns] } satisfies Settings;

      await this.store.save(updated, expectedRevision);

      return { settings: updated, apply: await this.reconcile(updated) };
    });
  }

  #serialize<T>(operation: () => Promise<T>): Promise<T> {
    const result = this.#queue.then(operation, operation);

    this.#queue = result.then(() => undefined, () => undefined);

    return result;
  }
}
