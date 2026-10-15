import { mkdtemp } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { describe, expect, it } from 'vitest';

import { DomainService } from '../../../src/server/domains/service';
import { SettingsStore } from '../../../src/server/domains/store';

describe('DomainService', () => {
  it('registers unique normalized domains and advances the revision', async () => {
    const store = new SettingsStore(await mkdtemp(join(tmpdir(), 'own-domains-service-')));
    const service = new DomainService(store, (settings) => Promise.resolve({ desiredRevision: settings.revision, appliedRevision: settings.revision, status: 'applied' }));

    const result = await service.create({ hostname: 'Shop.OWN.', upstreamPort: 3000, tls: false, enabled: true }, 0);

    expect(result.settings.revision).toBe(1);
    expect(result.settings.mappings[0]?.hostname).toBe('shop.own');
    expect(result.apply.status).toBe('applied');
  });

  it('rejects duplicate normalized domains', async () => {
    const store = new SettingsStore(await mkdtemp(join(tmpdir(), 'own-domains-service-')));
    const service = new DomainService(store, (settings) => Promise.resolve({ desiredRevision: settings.revision, appliedRevision: settings.revision, status: 'applied' }));

    await service.create({ hostname: 'shop.own', upstreamPort: 3000, tls: false, enabled: true }, 0);

    await expect(service.create({ hostname: 'SHOP.OWN.', upstreamPort: 3001, tls: false, enabled: true }, 1)).rejects.toThrow(/already registered/i);
  });

  it('updates and removes a mapping with revision checks', async () => {
    const store = new SettingsStore(await mkdtemp(join(tmpdir(), 'own-domains-service-')));
    const service = new DomainService(store, (settings) => Promise.resolve({ desiredRevision: settings.revision, appliedRevision: settings.revision, status: 'applied' }));
    const created = await service.create({ hostname: 'shop.own', upstreamPort: 3000, tls: false, enabled: true }, 0);
    const id = created.settings.mappings[0]!.id;

    const updated = await service.update(id, { upstreamPort: 3001, tls: true }, 1);

    expect(updated.settings.mappings[0]).toMatchObject({ upstreamPort: 3001, tls: true });

    const removed = await service.remove(id, 2);

    expect(removed.settings.mappings).toEqual([]);
    expect(removed.settings.revision).toBe(3);
  });

  it('generates IDs on create instead of accepting client IDs', async () => {
    const store = new SettingsStore(await mkdtemp(join(tmpdir(), 'own-domains-service-')));
    const service = new DomainService(store, (settings) => Promise.resolve({ desiredRevision: settings.revision, appliedRevision: settings.revision, status: 'applied' }));

    const first = await service.create({ id: 'shared', hostname: 'one.own', upstreamPort: 3000, tls: false, enabled: true }, 0);
    const second = await service.create({ id: 'shared', hostname: 'two.own', upstreamPort: 3001, tls: false, enabled: true }, 1);

    expect(new Set(second.settings.mappings.map((mapping) => mapping.id)).size).toBe(2);
    expect(first.settings.mappings[0]?.id).not.toBe('shared');
  });

  it('reconciles durable settings on demand', async () => {
    const store = new SettingsStore(await mkdtemp(join(tmpdir(), 'own-domains-service-')));
    const calls: number[] = [];
    const service = new DomainService(store, (settings) => {
      calls.push(settings.revision);

      return Promise.resolve({ desiredRevision: settings.revision, appliedRevision: settings.revision, status: 'applied' });
    });

    await service.create({ hostname: 'shop.own', upstreamPort: 3000, tls: false, enabled: true }, 0);

    await expect(service.reconcileCurrent()).resolves.toMatchObject({ desiredRevision: 1, appliedRevision: 1 });
    expect(calls).toEqual([1, 1]);
  });

  it('updates validated LAN and upstream DNS settings', async () => {
    const store = new SettingsStore(await mkdtemp(join(tmpdir(), 'own-domains-service-')));
    const service = new DomainService(store, (settings) => Promise.resolve({ desiredRevision: settings.revision, appliedRevision: settings.revision, status: 'applied' }));

    const result = await service.updateNetwork({ lanAddress: '192.168.1.20', upstreamDns: ['1.1.1.1'] }, 0);

    expect(result.settings).toMatchObject({ revision: 1, lanAddress: '192.168.1.20', upstreamDns: ['1.1.1.1'] });
    await expect(service.updateNetwork({ lanAddress: 'not-an-ip', upstreamDns: ['1.1.1.1'] }, 1)).rejects.toThrow(/IPv4/i);
  });
});
