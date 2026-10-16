import { describe, expect, it } from 'vitest';

import { buildLaunchdActivationCommands, buildLaunchdDeactivationCommands } from '../../src/cli/setup-services';

describe('buildLaunchdActivationCommands', () => {
  it('boots infrastructure as system services and the backend in the user session', () => {
    expect(buildLaunchdActivationCommands('/Users/kien', 501)).toEqual([
      { command: '/bin/launchctl', args: ['bootout', 'system/dev.owndomains.dnsmasq'], ignoreFailure: true },
      { command: '/bin/launchctl', args: ['bootout', 'system/dev.owndomains.caddy'], ignoreFailure: true },
      { command: '/bin/launchctl', args: ['asuser', '501', '/bin/launchctl', 'bootout', 'gui/501/dev.owndomains.backend'], ignoreFailure: true },
      { command: '/bin/launchctl', args: ['bootstrap', 'system', '/Library/LaunchDaemons/dev.owndomains.dnsmasq.plist'], ignoreFailure: false },
      { command: '/bin/launchctl', args: ['bootstrap', 'system', '/Library/LaunchDaemons/dev.owndomains.caddy.plist'], ignoreFailure: false },
      { command: '/bin/launchctl', args: ['asuser', '501', '/bin/launchctl', 'bootstrap', 'gui/501', '/Users/kien/Library/LaunchAgents/dev.owndomains.backend.plist'], ignoreFailure: false },
    ]);
  });

  it('unloads every managed service after partial activation', () => {
    expect(buildLaunchdDeactivationCommands(501)).toEqual([
      { command: '/bin/launchctl', args: ['asuser', '501', '/bin/launchctl', 'bootout', 'gui/501/dev.owndomains.backend'], ignoreFailure: true },
      { command: '/bin/launchctl', args: ['bootout', 'system/dev.owndomains.caddy'], ignoreFailure: true },
      { command: '/bin/launchctl', args: ['bootout', 'system/dev.owndomains.dnsmasq'], ignoreFailure: true },
    ]);
  });
});
