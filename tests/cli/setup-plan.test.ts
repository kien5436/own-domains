import { describe, expect, it } from 'vitest';

import { buildSetupPlan } from '../../src/cli/setup-plan';

const options = {
  brewPrefix: '/opt/homebrew',
  homeDirectory: '/Users/kien',
  repositoryRoot: '/Users/kien/workspace/domains',
  userName: 'kien',
  lanAddress: '192.168.1.20',
  upstreamDns: ['1.1.1.1', '9.9.9.9'],
};

describe('buildSetupPlan', () => {
  it('creates fixed resolver, service, and manifest artifacts', () => {
    const plan = buildSetupPlan(options);

    expect(plan.artifacts.map((artifact) => artifact.path)).toEqual([
      '/etc/resolver/own',
      '/opt/homebrew/etc/dnsmasq.d/own-domains.conf',
      '/Users/kien/Library/Application Support/OwnDomains/caddy-bootstrap.json',
      '/Library/LaunchDaemons/dev.owndomains.dnsmasq.plist',
      '/Library/LaunchDaemons/dev.owndomains.caddy.plist',
      '/Users/kien/Library/LaunchAgents/dev.owndomains.backend.plist',
      '/Users/kien/Library/Application Support/OwnDomains/install-manifest.json',
    ]);
    expect(new Set(plan.artifacts.map((artifact) => artifact.path)).size).toBe(plan.artifacts.length);
  });

  it('records every owned system artifact for reversible removal', () => {
    const plan = buildSetupPlan(options);
    const manifest = JSON.parse(plan.artifacts.at(-1)?.content ?? '{}') as { schemaVersion: number; ownedPaths: string[] };

    expect(manifest.schemaVersion).toBe(1);
    expect(manifest.ownedPaths).toEqual(plan.artifacts.slice(0, -1).map((artifact) => artifact.path));
    expect(plan.artifacts.slice(0, -1).every((artifact) => artifact.backupExisting)).toBe(true);
  });

  it('keeps generated service commands independent of browser input', () => {
    const plan = buildSetupPlan(options);
    const contents = plan.artifacts.map((artifact) => artifact.content).join('\n');

    expect(contents).toContain('/opt/homebrew/sbin/dnsmasq');
    expect(contents).toContain('/opt/homebrew/bin/caddy');
    expect(contents).toContain('/opt/homebrew/bin/pnpm');
    expect(contents).toContain('/Users/kien/workspace/domains');
  });
});
