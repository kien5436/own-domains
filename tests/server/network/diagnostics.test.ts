import { describe, expect, it } from 'vitest';

import { diagnoseNetwork } from '../../../src/server/network/diagnostics';

describe('diagnoseNetwork', () => {
  it('reports an actionable warning when no LAN IPv4 is selected', async () => {
    const diagnostics = await diagnoseNetwork({ lanAddress: '', mappings: [] }, () => Promise.resolve(true));

    expect(diagnostics).toContainEqual(expect.objectContaining({ code: 'lan-address', status: 'warning', remedy: expect.stringMatching(/LAN/i) }));
  });

  it('reports unreachable upstream applications', async () => {
    const diagnostics = await diagnoseNetwork({ lanAddress: '192.168.1.20', mappings: [{ id: '1', hostname: 'shop.own', upstreamPort: 3000, tls: false, enabled: true }] }, () => Promise.resolve(false));

    expect(diagnostics).toContainEqual(expect.objectContaining({ code: 'upstream:shop.own', status: 'error' }));
  });

  it('allows WARP DNS on separate loopback addresses when .own fallback is configured', async () => {
    const inspect = () => Promise.resolve({
      dnsConfiguration: 'nameserver[0] : 127.0.2.2',
      port53Listeners: 'dnsmasq 10 user UDP 127.0.0.1:53\ndnsmasq 10 user UDP 192.168.1.20:53\nCloudflareWARP 20 root UDP 127.0.2.2:53',
      warpSettings: 'Local Domain Fallback: own -> 192.168.1.20',
    });
    const diagnostics = await diagnoseNetwork({ lanAddress: '192.168.1.20', mappings: [] }, () => Promise.resolve(true), inspect);

    expect(diagnostics).toContainEqual(expect.objectContaining({ code: 'dns-port', status: 'ok' }));
    expect(diagnostics).toContainEqual(expect.objectContaining({ code: 'warp-dns', status: 'ok' }));
  });

  it('reports a service occupying DNS on all interfaces', async () => {
    const inspect = () => Promise.resolve({ dnsConfiguration: '', port53Listeners: 'OtherDNS 30 root UDP *:53', warpSettings: '' });
    const diagnostics = await diagnoseNetwork({ lanAddress: '192.168.1.20', mappings: [] }, () => Promise.resolve(true), inspect);

    expect(diagnostics).toContainEqual(expect.objectContaining({ code: 'dns-port', status: 'error', message: expect.stringContaining('OtherDNS') }));
  });

  it('warns when WARP DNS is active without a confirmed .own fallback', async () => {
    const inspect = () => Promise.resolve({ dnsConfiguration: 'nameserver[0] : 127.0.2.2', port53Listeners: 'CloudflareWARP 20 root UDP 127.0.2.2:53', warpSettings: 'Local Domain Fallback: example.internal' });
    const diagnostics = await diagnoseNetwork({ lanAddress: '192.168.1.20', mappings: [] }, () => Promise.resolve(true), inspect);

    expect(diagnostics).toContainEqual(expect.objectContaining({ code: 'warp-dns', status: 'warning', remedy: expect.stringMatching(/fallback/i) }));
  });
});
