import { copyFileSync, mkdirSync, statSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';

export function stagedDaemonName(target: string): string {
  return `own-domains-daemon-${target}${target.includes('-windows-') ? '.exe' : ''}`;
}

function hostTriple(): string {
  const result = spawnSync('rustc', ['-vV'], { encoding: 'utf8' });

  if (0 !== result.status) throw new Error('rustc -vV failed');
  const match = /^host: (\S+)$/m.exec(result.stdout);

  if (!match) throw new Error('rustc did not report a host target');

  return match[1];
}

function main(): number {
  const argv = process.argv.slice(2);
  const release = argv.includes('--release');
  const targetIndex = argv.indexOf('--target');
  const target = -1 === targetIndex ? hostTriple() : argv[targetIndex + 1];

  if (!target) throw new Error('--target requires a triple');
  const profile = release ? 'release' : 'debug';
  const workspace = resolve(process.cwd());
  const executable = `own-domains-daemon${target.includes('-windows-') ? '.exe' : ''}`;
  const source = join(
    workspace,
    'target',
    ...(-1 === targetIndex ? [] : [target]),
    profile,
    executable,
  );
  const destinationDirectory = join(
    workspace,
    'crates',
    'own-domains-desktop',
    'binaries',
  );
  const destination = join(destinationDirectory, stagedDaemonName(target));

  mkdirSync(destinationDirectory, { recursive: true });
  copyFileSync(source, destination);

  // Mode bits are meaningless on Windows; the suffix above is the contract.
  if ('win32' !== process.platform) {
    const mode = statSync(destination).mode & 0o777;

    if (0 === (mode & 0o111)) throw new Error(`staged daemon is not executable: ${destination}`);
  }

  process.stdout.write(`staged ${destination}\n`);

  return 0;
}

try {
  process.exitCode = main();
}
catch (error) {
  process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
  process.exitCode = 1;
}
