import { describe, expect, it } from 'vitest';

import { buildSetupPlan } from '../../src/cli/setup-plan';

const options = {
  backupDirectory: '/Users/kien/Library/Application Support/OwnDomains/backups/setup-1',
  brewPrefix: '/opt/homebrew',
  homeDirectory: '/Users/kien',
  repositoryRoot: '/Users/kien/workspace/domains',
  userId: 501,
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
    const manifest = JSON.parse(plan.artifacts.at(-1)?.content ?? '{}') as { schemaVersion: number; backupDirectory: string; artifacts: { path: string; sha256: string }[] };

    expect(manifest.schemaVersion).toBe(1);
    expect(manifest.backupDirectory).toBe(options.backupDirectory);
    expect(manifest.artifacts.map((artifact) => artifact.path)).toEqual(plan.artifacts.slice(0, -1).map((artifact) => artifact.path));
    expect(manifest.artifacts.every((artifact) => /^[a-f\d]{64}$/.test(artifact.sha256))).toBe(true);
    expect(plan.artifacts.slice(0, -1).every((artifact) => artifact.backupExisting)).toBe(true);
  });

  it('keeps generated service commands independent of browser input', () => {
    const plan = buildSetupPlan(options);
    const contents = plan.artifacts.map((artifact) => artifact.content).join('\n');

    expect(contents).toContain('/opt/homebrew/sbin/dnsmasq');
    expect(contents).toContain('/opt/homebrew/bin/caddy');
    expect(plan.artifacts.find((item) => item.path.endsWith('caddy-bootstrap.json'))?.content).toContain('127.0.0.1:2020');
    expect(contents).toContain('/opt/homebrew/bin/pnpm');
    expect(contents).toContain('/Users/kien/workspace/domains');
    expect(contents).toContain('<key>PATH</key>\n      <string>/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin</string>');
  });

  it('keeps system files root-owned and application files user-owned', () => {
    const plan = buildSetupPlan(options);
    const owners = new Map(plan.artifacts.map((artifact) => [artifact.path, artifact.ownerId]));

    expect(owners.get('/etc/resolver/own')).toBe(0);
    expect(owners.get('/Library/LaunchDaemons/dev.owndomains.caddy.plist')).toBe(0);
    expect(owners.get('/opt/homebrew/etc/dnsmasq.d/own-domains.conf')).toBe(501);
    expect(owners.get('/Users/kien/Library/LaunchAgents/dev.owndomains.backend.plist')).toBe(501);
    expect(owners.get('/Users/kien/Library/Application Support/OwnDomains/install-manifest.json')).toBe(501);
  });
});
