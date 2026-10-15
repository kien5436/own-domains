import { isIP } from 'node:net';

import type { Settings } from '../../shared/types';

type DnsConfigOptions = { port?: number; listenAddresses?: string[]; upstreamPort?: number };

export function buildDnsConfig(settings: Settings, options: DnsConfigOptions = {}): string {
  if (4 !== isIP(settings.lanAddress)) throw new Error('A valid LAN IPv4 address is required');
  if (settings.upstreamDns.some((address) => address === settings.lanAddress || '127.0.0.1' === address)) throw new Error('DNS forwarding loop detected');
  if (!settings.upstreamDns.every((address) => 4 === isIP(address))) throw new Error('Upstream DNS must use IPv4 addresses');
  const { port = 53, listenAddresses = ['127.0.0.1', settings.lanAddress], upstreamPort } = options;

  return [
    `port=${port}`,
    'no-resolv',
    'no-hosts',
    `listen-address=${listenAddresses.join(',')}`,
    'bind-interfaces',
    'local-service=net',
    `address=/.own/${settings.lanAddress}`,
    'local=/.own/',
    ...settings.upstreamDns.map((address) => `server=${address}${upstreamPort ? `#${upstreamPort}` : ''}`),
    'no-dhcp-interface=*',
    '',
  ].join('\n');
}
