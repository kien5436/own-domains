import { spawnSync } from 'node:child_process';

export function commandExists(command: string): boolean {
  return 0 === spawnSync('/usr/bin/which', [command], { stdio: 'ignore' }).status;
}
