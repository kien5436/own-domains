import { mkdtemp } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';

import { DomainService } from '../../../src/server/domains/service';
import { SettingsStore } from '../../../src/server/domains/store';
import { createManagementServer } from '../../../src/server/http/server';

const servers: Array<{ close: () => Promise<void> }> = [];

afterEach(() => Promise.all(servers.splice(0).map((server) => server.close())));

async function start() {
  const store = new SettingsStore(await mkdtemp(join(tmpdir(), 'own-domains-api-')));
  const service = new DomainService(store, (settings) => Promise.resolve({ desiredRevision: settings.revision, appliedRevision: settings.revision, status: 'applied' }));
  const server = await createManagementServer({ service, store, host: '127.0.0.1', port: 0 });

  servers.push(server);

  return server;
}

describe('management API', () => {
  it('creates and lists mappings with CSRF protection', async () => {
    const server = await start();
    const session = await fetch(`${server.origin}/api/session`);
    const csrf = (await session.json() as { csrfToken: string }).csrfToken;
    const cookie = session.headers.get('set-cookie')!;

    const created = await fetch(`${server.origin}/api/domains`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', cookie, 'x-csrf-token': csrf, origin: server.origin },
      body: JSON.stringify({ expectedRevision: 0, mapping: { hostname: 'shop.own', upstreamPort: 3000, tls: false, enabled: true } }),
    });

    expect(created.status).toBe(201);

    const listed = await fetch(`${server.origin}/api/domains`, { headers: { cookie } });

    expect(await listed.json()).toMatchObject({ revision: 1, mappings: [{ hostname: 'shop.own' }] });
  });

  it('rejects cross-origin mutations and missing CSRF tokens', async () => {
    const server = await start();
    const response = await fetch(`${server.origin}/api/domains`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', origin: 'https://hostile.example' },
      body: JSON.stringify({}),
    });

    expect(response.status).toBe(403);
  });

  it('returns accepted when desired settings persist but proxy apply fails', async () => {
    const store = new SettingsStore(await mkdtemp(join(tmpdir(), 'own-domains-api-')));
    const service = new DomainService(store, (settings) => Promise.resolve({ desiredRevision: settings.revision, appliedRevision: 0, status: 'error', message: 'Caddy unavailable' }));
    const server = await createManagementServer({ service, store, host: '127.0.0.1', port: 0 });

    servers.push(server);
    const session = await fetch(`${server.origin}/api/session`);
    const csrf = (await session.json() as { csrfToken: string }).csrfToken;
    const cookie = session.headers.get('set-cookie')!;

    const response = await fetch(`${server.origin}/api/domains`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', cookie, 'x-csrf-token': csrf, origin: server.origin },
      body: JSON.stringify({ expectedRevision: 0, mapping: { hostname: 'shop.own', upstreamPort: 3000, tls: false, enabled: true } }),
    });

    expect(response.status).toBe(202);
    expect(await response.json()).toMatchObject({ apply: { desiredRevision: 1, appliedRevision: 0, status: 'error' } });
  });
});
