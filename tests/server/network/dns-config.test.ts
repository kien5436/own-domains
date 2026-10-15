import { describe, expect, it } from 'vitest';

import { buildDnsConfig } from '../../../src/server/network/dns-config';
import type { Settings } from '../../../src/shared/types';

const base: Settings = {
  schemaVersion: 1,
  revision: 1,
  lanAddress: '192.168.1.20',
  upstreamDns: ['1.1.1.1', '9.9.9.9'],
  mappings: [],
};

describe('buildDnsConfig', () => {
  it('keeps .own local and forwards public names to explicit upstreams', () => {
    expect(buildDnsConfig(base)).toBe([
      'port=53',
      'no-resolv',
      'no-hosts',
      'listen-address=127.0.0.1,192.168.1.20',
      'bind-interfaces',
      'local-service=net',
      'address=/.own/192.168.1.20',
      'local=/.own/',
      'server=1.1.1.1',
      'server=9.9.9.9',
      'no-dhcp-interface=*',
      '',
    ].join('\n'));
  });

  it('rejects an upstream that loops to the DNS host', () => {
    expect(() => buildDnsConfig({ ...base, upstreamDns: ['192.168.1.20'] })).toThrow(/loop/i);
  });

  it('supports isolated listener and upstream ports for integration tests', () => {
    const config = buildDnsConfig({ ...base, upstreamDns: ['127.0.0.2'] }, { port: 15353, listenAddresses: ['127.0.0.1'], upstreamPort: 15354 });

    expect(config).toContain('port=15353\n');
    expect(config).toContain('listen-address=127.0.0.1\n');
    expect(config).toContain('server=127.0.0.2#15354\n');
  });
});
