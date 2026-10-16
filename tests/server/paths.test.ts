import { describe, expect, it } from 'vitest';

import { caddyRootCertificatePath } from '../../src/server/paths';

describe('caddyRootCertificatePath', () => {
  it('matches the dedicated Caddy data directory installed by setup', () => {
    expect(caddyRootCertificatePath('/Users/kien/Library/Application Support/OwnDomains')).toBe('/Users/kien/Library/Application Support/OwnDomains/caddy-data/caddy/pki/authorities/local/root.crt');
  });
});
