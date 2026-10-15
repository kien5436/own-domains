import { describe, expect, it } from 'vitest';

import { normalizeHostname, validateMapping } from '../../../src/server/domains/validation';

describe('normalizeHostname', () => {
  it('normalizes case and one trailing dot', () => {
    expect(normalizeHostname('Shop.OWN.')).toBe('shop.own');
  });

  it('accepts nested explicit subdomains', () => {
    expect(normalizeHostname('admin.api.shop.own')).toBe('admin.api.shop.own');
  });
});

describe('validateMapping', () => {
  it.each(['own', 'setup.own', 'child.setup.own', '*.shop.own', 'café.own', 'bad..own', '-bad.own', 'bad-.own'])('rejects invalid hostname %s', (hostname) => {
    expect(() => validateMapping({ hostname, upstreamPort: 3000, tls: false, enabled: true })).toThrow();
  });

  it.each([53, 80, 443, 4780, 0, 65536])('rejects reserved or invalid port %s', (upstreamPort) => {
    expect(() => validateMapping({ hostname: 'shop.own', upstreamPort, tls: false, enabled: true })).toThrow();
  });

  it('returns a normalized mapping with a stable supplied id', () => {
    expect(validateMapping({ id: 'map-1', hostname: 'Shop.OWN.', upstreamPort: 3000, tls: true, enabled: true })).toEqual({
      id: 'map-1',
      hostname: 'shop.own',
      upstreamPort: 3000,
      tls: true,
      enabled: true,
    });
  });
});
