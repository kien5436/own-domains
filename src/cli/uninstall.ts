import { homedir } from 'node:os';
import { join } from 'node:path';

import { executeUninstall, parseUninstallArguments } from './uninstall-command';

try {
  const uninstallArguments = parseUninstallArguments(process.argv.slice(2));
  const homeDirectory = process.env.SUDO_USER ? join('/Users', process.env.SUDO_USER) : homedir();
  const manifestPath = join(homeDirectory, 'Library/Application Support/OwnDomains/install-manifest.json');

  if (uninstallArguments.apply && 0 !== process.getuid?.()) throw new Error('Apply requires administrator approval. Re-run this command with sudo.');
  const result = await executeUninstall({ apply: uninstallArguments.apply, manifestPath });

  process.stdout.write(result.stdout);
  process.stderr.write(result.stderr);
  process.exitCode = result.exitCode;
}
catch (error) {
  process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
  process.exitCode = 2;
}
