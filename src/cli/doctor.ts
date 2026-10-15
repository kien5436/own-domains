import { access } from 'node:fs/promises';
import { constants } from 'node:fs';

import { commandExists } from './commands';

const checks = [
  ['caddy', 'Caddy reverse proxy'],
  ['dnsmasq', 'dnsmasq local DNS'],
] as const;

let failed = false;

for (const [command, label] of checks) {
  const ok = commandExists(command);

  process.stdout.write(`${ok ? 'OK' : 'MISSING'} ${label}\n`);
  failed ||= !ok;
}

const resolver = '/etc/resolver/own';
const hasResolver = await access(resolver, constants.R_OK).then(() => true, () => false);

process.stdout.write(`${hasResolver ? 'OK' : 'MISSING'} ${resolver}\n`);
failed ||= !hasResolver;
process.exitCode = failed ? 1 : 0;
