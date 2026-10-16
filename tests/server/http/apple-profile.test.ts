import { describe, expect, it } from 'vitest';

import { buildAppleCaProfile } from '../../../src/server/http/apple-profile';

const certificate = '-----BEGIN CERTIFICATE-----\nY2VydGlmaWNhdGUtYnl0ZXM=\n-----END CERTIFICATE-----\n';

describe('buildAppleCaProfile', () => {
  it('wraps the DER certificate in a stable Apple root-certificate profile', () => {
    const first = buildAppleCaProfile(certificate);
    const second = buildAppleCaProfile(certificate);

    expect(first).toBe(second);
    expect(first).toContain('<string>Configuration</string>');
    expect(first).toContain('<string>com.apple.security.root</string>');
    expect(first).toContain('<string>own-domains-ca.cer</string>');
    expect(first).toContain('<data>Y2VydGlmaWNhdGUtYnl0ZXM=</data>');
    expect(first).toMatch(/<key>PayloadUUID<\/key>\s*<string>[A-F\d]{8}(?:-[A-F\d]{4}){3}-[A-F\d]{12}<\/string>/);
  });

  it('rejects malformed certificate input', () => {
    expect(() => buildAppleCaProfile('not a certificate')).toThrow(/PEM certificate/i);
  });
});
