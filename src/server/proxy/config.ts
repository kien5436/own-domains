import { isIP } from 'node:net';

import type { Settings } from '../../shared/types';

type CaddyRoute = {
  match?: Array<{ host: string[] }>;
  handle: Array<Record<string, unknown>>;
  terminal: boolean;
};

type ProxyConfigOptions = {
  adminListen?: string;
  addresses?: string[];
  httpPort?: number;
  httpsPort?: number;
  setupPort?: number;
};

export function buildProxyConfig(settings: Settings, options: ProxyConfigOptions = {}) {
  if (4 !== isIP(settings.lanAddress)) throw new Error('A valid LAN IPv4 address is required');
  const { adminListen = 'unix//tmp/own-domains-caddy.sock', addresses = ['127.0.0.1', settings.lanAddress], httpPort = 80, httpsPort = 443, setupPort = 4781 } = options;
  const enabled = settings.mappings.filter((mapping) => mapping.enabled);
  const setupRoute = proxyRouteForHosts(['setup.own', settings.lanAddress], setupPort);
  const httpRoutes: CaddyRoute[] = [
    setupRoute,
    ...enabled.map((mapping) => mapping.tls
      ? {
        match: [{ host: [mapping.hostname] }],
        handle: [{ handler: 'static_response', headers: { Location: [`https://${mapping.hostname}{http.request.uri}`] }, status_code: 308 }],
        terminal: true,
      }
      : proxyRoute(mapping.hostname, mapping.upstreamPort)),
    notFoundRoute(),
  ];
  const tlsMappings = enabled.filter((mapping) => mapping.tls);

  return {
    admin: { listen: adminListen },
    apps: {
      http: {
        http_port: httpPort,
        https_port: httpsPort,
        servers: {
          http: { listen: addresses.map((address) => `${address}:${httpPort}`), routes: httpRoutes },
          https: { listen: addresses.map((address) => `${address}:${httpsPort}`), routes: [...tlsMappings.map((mapping) => proxyRoute(mapping.hostname, mapping.upstreamPort)), notFoundRoute()], tls_connection_policies: [{}], automatic_https: { disable_redirects: true } },
        },
      },
      tls: { automation: { policies: tlsMappings.map((mapping) => ({ subjects: [mapping.hostname], issuers: [{ module: 'internal' }] })) } },
    },
  };
}

function notFoundRoute(): CaddyRoute {
  return { handle: [{ handler: 'static_response', body: 'Unknown .own domain', status_code: 404 }], terminal: true };
}

function proxyRoute(hostname: string, port: number): CaddyRoute {
  return proxyRouteForHosts([hostname], port);
}

function proxyRouteForHosts(hostnames: string[], port: number): CaddyRoute {
  return {
    match: [{ host: hostnames }],
    handle: [{ handler: 'reverse_proxy', upstreams: [{ dial: `127.0.0.1:${port}` }] }],
    terminal: true,
  };
}
