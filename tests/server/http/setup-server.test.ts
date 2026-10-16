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

  it('serves an Apple configuration profile and links it from setup', async () => {
    const certificate = '-----BEGIN CERTIFICATE-----\nY2VydGlmaWNhdGUtYnl0ZXM=\n-----END CERTIFICATE-----\n';
    const server = await createSetupServer({ host: '127.0.0.1', port: 0, readCaCertificate: () => Promise.resolve(certificate) });

    closeCallbacks.push(server.close);
    const profile = await fetch(`${server.origin}/ca.mobileconfig`);
    const setup = await fetch(server.origin);

    expect(profile.status).toBe(200);
    expect(profile.headers.get('content-type')).toBe('application/x-apple-aspen-config');
    expect(profile.headers.get('content-disposition')).toContain('own-domains-ca.mobileconfig');
    expect(await profile.text()).toContain('<string>com.apple.security.root</string>');
    expect(await setup.text()).toContain('href="/ca.mobileconfig"');
  });
});
