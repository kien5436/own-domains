import { describe, expect, it } from 'vitest';

import { buildProxyConfig } from '../../../src/server/proxy/config';
import type { Settings } from '../../../src/shared/types';

const settings: Settings = {
  schemaVersion: 1,
  revision: 3,
  lanAddress: '192.168.1.20',
  upstreamDns: ['1.1.1.1'],
  mappings: [
    { id: 'shop', hostname: 'shop.own', upstreamPort: 3000, tls: false, enabled: true },
    { id: 'api', hostname: 'api.shop.own', upstreamPort: 8080, tls: true, enabled: true },
    { id: 'off', hostname: 'off.own', upstreamPort: 9000, tls: false, enabled: false },
  ],
};

describe('buildProxyConfig', () => {
  it('creates exact HTTP routes only for enabled mappings', () => {
    const config = buildProxyConfig(settings);

    expect(config.admin.listen).toBe('127.0.0.1:2020');
    expect(config.apps.http.servers.http.listen).toEqual(['127.0.0.1:80', '192.168.1.20:80']);
    expect(config.apps.http.servers.https.listen).toEqual(['127.0.0.1:443', '192.168.1.20:443']);
    expect(config.apps.http.servers.http.routes).toEqual([
      {
        match: [{ host: ['setup.own', '192.168.1.20'] }],
        handle: [{ handler: 'reverse_proxy', upstreams: [{ dial: '127.0.0.1:4781' }] }],
        terminal: true,
      },
      {
        match: [{ host: ['shop.own'] }],
        handle: [{ handler: 'reverse_proxy', upstreams: [{ dial: '127.0.0.1:3000' }] }],
        terminal: true,
      },
      {
        match: [{ host: ['api.shop.own'] }],
        handle: [{ handler: 'static_response', headers: { Location: ['https://api.shop.own{http.request.uri}'] }, status_code: 308 }],
        terminal: true,
      },
      {
        handle: [{ handler: 'static_response', body: 'Unknown .own domain', status_code: 404 }],
        terminal: true,
      },
    ]);
  });

  it('rejects proxy configuration without a valid LAN IPv4 address', () => {
    expect(() => buildProxyConfig({ ...settings, lanAddress: '' })).toThrow(/LAN IPv4/i);
  });

  it('uses Caddy internal TLS only for TLS mappings', () => {
    const config = buildProxyConfig(settings);

    expect(config.apps.tls.automation.policies).toEqual([{ subjects: ['api.shop.own'], issuers: [{ module: 'internal' }] }]);
    expect(config.apps.http.servers.https.routes[0]?.match).toEqual([{ host: ['api.shop.own'] }]);
  });

  it('supports isolated listeners and an admin socket for integration tests', () => {
    const config = buildProxyConfig(settings, { adminListen: 'unix//tmp/own-domains-test.sock', addresses: ['127.0.0.1'], httpPort: 18080, httpsPort: 18443, setupPort: 14781 });

    expect(config.admin.listen).toBe('unix//tmp/own-domains-test.sock');
    expect(config.apps.http.servers.http.listen).toEqual(['127.0.0.1:18080']);
    expect(config.apps.http.servers.https.listen).toEqual(['127.0.0.1:18443']);
    expect(config.apps.http.servers.http.routes[0]?.handle).toEqual([{ handler: 'reverse_proxy', upstreams: [{ dial: '127.0.0.1:14781' }] }]);
  });
});
