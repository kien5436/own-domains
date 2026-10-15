import { homedir, userInfo } from 'node:os';
import { join } from 'node:path';

import { executeSetup, parseSetupArguments } from './setup-command';
import { buildSetupPlan } from './setup-plan';
import { inspectSetupPlan } from './setup-preflight';
import { activateLaunchdServices } from './setup-services';

try {
  const setupArguments = parseSetupArguments(process.argv.slice(2));
  const userName = process.env.SUDO_USER ?? userInfo().username;
  const userId = Number(process.env.SUDO_UID ?? process.getuid?.() ?? 0);
  const homeDirectory = process.env.SUDO_USER ? join('/Users', process.env.SUDO_USER) : homedir();
  const applicationDirectory = join(homeDirectory, 'Library/Application Support/OwnDomains');
  const backupDirectory = join(applicationDirectory, 'backups', new Date().toISOString().replaceAll(':', '-'));
  const plan = buildSetupPlan({
    backupDirectory,
    brewPrefix: process.env.HOMEBREW_PREFIX ?? ('arm64' === process.arch ? '/opt/homebrew' : '/usr/local'),
    homeDirectory,
    repositoryRoot: process.cwd(),
    userId,
    userName,
    lanAddress: setupArguments.lanAddress,
    upstreamDns: setupArguments.upstreamDns,
  });

  if (setupArguments.apply && 0 !== process.getuid?.()) throw new Error('Apply requires administrator approval. Re-run this command with sudo.');
  const result = await executeSetup(
    { apply: setupArguments.apply, backupDirectory, plan },
    {
      activate: () => activateLaunchdServices(homeDirectory, userId),
      inspect: (setupPlan) => inspectSetupPlan(setupPlan, { listenAddresses: ['127.0.0.1', setupArguments.lanAddress], userId }),
    },
  );

  process.stdout.write(result.stdout);
  process.stderr.write(result.stderr);
  process.exitCode = result.exitCode;
}
catch (error) {
  process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
  process.exitCode = 2;
}
