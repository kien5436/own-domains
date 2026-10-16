import { afterEach, describe, expect, it } from 'vitest';

import { createSetupServer } from '../../../src/server/http/setup-server';

const closeCallbacks: Array<() => Promise<void>> = [];

afterEach(async () => {
  await Promise.all(closeCallbacks.splice(0).map((close) => close()));
});

describe('createSetupServer', () => {
  it('downloads the public CA returned by the local Caddy client', async () => {
    const certificate = '-----BEGIN CERTIFICATE-----\ntest\n-----END CERTIFICATE-----\n';
    const server = await createSetupServer({ host: '127.0.0.1', port: 0, readCaCertificate: () => Promise.resolve(certificate) });

    closeCallbacks.push(server.close);
    const response = await fetch(`${server.origin}/ca.crt`);

    expect(response.status).toBe(200);
    expect(response.headers.get('content-disposition')).toContain('own-domains-ca.crt');
    expect(await response.text()).toBe(certificate);
  });
});
