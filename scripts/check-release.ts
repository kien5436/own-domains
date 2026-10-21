import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';

import {
  type Architecture,
  type RuntimeSidecarManifest,
  type SidecarBuildManifest,
  type SidecarName,
  verifySidecars,
} from './fetch-sidecars';

const ARCHITECTURES: Array<{ name: Architecture; target: string; runArch: string }> = [
  { name: 'arm64', target: 'aarch64-apple-darwin', runArch: 'arm64' },
  { name: 'amd64', target: 'x86_64-apple-darwin', runArch: 'x86_64' },
];

function run(command: string, args: string[], cwd?: string): string {
  const result = spawnSync(command, args, {
    cwd,
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
  });

  if (result.error) throw result.error;

  if (0 !== result.status)
    throw new Error(
      `${command} ${args.join(' ')} failed with ${result.status}: ${result.stderr.trim()}`,
    );

  return `${result.stdout ?? ''}${result.stderr ?? ''}`;
}

function digest(path: string): string {
  return createHash('sha256').update(readFileSync(path)).digest('hex');
}

function parseManifest(path: string): SidecarBuildManifest {
  return JSON.parse(readFileSync(path, 'utf8')) as SidecarBuildManifest;
}

function runVersion(path: string, component: SidecarName, architecture: string): string {
  const args = 'coredns' === component ? ['-version'] : ['version'];
  const result = spawnSync('/usr/bin/arch', [`-${architecture}`, path, ...args], {
    encoding: 'utf8',
    timeout: 15_000,
  });

  if (result.error) throw result.error;

  if (0 !== result.status)
    throw new Error(
      `${component} version command failed with ${result.status}: ${result.stderr.trim()}`,
    );

  return `${result.stdout ?? ''}${result.stderr ?? ''}`;
}

async function verifyPreflight(): Promise<void> {
  const manifest = parseManifest(join(process.cwd(), 'sidecars', 'manifest.json'));
  const verified = await verifySidecars({
    manifest,
    outputDirectory: join(process.cwd(), 'sidecars', 'build'),
    architectures: ARCHITECTURES.map((architecture) => architecture.name),
  });

  for (const [index, sidecar] of verified.entries())
    process.stdout.write(
      `${JSON.stringify({
        event: 'release-preflight',
        architecture: ARCHITECTURES[index]?.name,
        target: sidecar.target,
        coredns: sidecar.corednsPath,
        caddy: sidecar.caddyPath,
        status: 'passed',
      })}\n`,
    );

}

function verifyVersion(
  manifest: SidecarBuildManifest,
  architecture: Architecture,
  path: string,
  component: SidecarName,
): void {
  const expected = 'coredns' === component ? manifest.coredns.version : manifest.caddy.version;
  const runArch = ARCHITECTURES.find((item) => item.name === architecture)?.runArch;

  if (!runArch) throw new Error(`unsupported architecture: ${architecture}`);
  const output = runVersion(path, component, runArch);

  if (!output.includes(expected))
    throw new Error(
      `${component} version mismatch: expected ${expected}, received ${output.trim()}`,
    );

}

function verifyApp(app: string, target: string, manifestPath: string): void {
  const contents = join(app, 'Contents');
  const macos = join(contents, 'MacOS');
  const resources = join(contents, 'Resources');
  const runtimeManifestPath = join(resources, 'sidecars', `manifest-${target}.json`);

  if (!existsSync(app)) throw new Error(`signed app is missing: ${app}`);

  if (!existsSync(runtimeManifestPath))
    throw new Error(`runtime sidecar manifest is missing: ${runtimeManifestPath}`);

  const runtime = JSON.parse(readFileSync(runtimeManifestPath, 'utf8')) as RuntimeSidecarManifest;
  const build = parseManifest(manifestPath);
  const architecture = ARCHITECTURES.find((item) => item.target === target)?.name;

  if (!architecture) throw new Error(`unsupported target: ${target}`);

  if (
    1 !== runtime.schemaVersion
    || runtime.coredns.version !== build.coredns.version
    || runtime.caddy.version !== build.caddy.version
    || runtime.coredns.sha256 !== build.coredns.releases[architecture].executableSha256
    || runtime.caddy.sha256 !== build.caddy.releases[architecture].executableSha256
  )
    throw new Error(`runtime manifest in ${app} does not match the pinned build manifest`);

  for (const [name, filename, expected] of [
    ['CoreDNS', `coredns-${target}`, runtime.coredns.sha256],
    ['Caddy', `caddy-${target}`, runtime.caddy.sha256],
  ] as const) {
    const path = join(resources, filename);

    if (!existsSync(path)) throw new Error(`${name} is missing from the signed app: ${path}`);
    const mode = statSync(path).mode & 0o777;

    if (0 !== (mode & 0o022)) throw new Error(`${name} is writable after signing: ${path}`);
    if (0 === (mode & 0o111)) throw new Error(`${name} is not executable: ${path}`);

    const actual = digest(path);

    if (actual !== expected)
      throw new Error(
        `${name} digest mismatch in the signed app: expected ${expected}, received ${actual}`,
      );

  }

  for (const filename of ['own-domains-daemon', 'own-domains-desktop']) {
    const path = join(macos, filename);

    if (!existsSync(path)) throw new Error(`signed app is missing ${filename}: ${path}`);

    if (0 !== (statSync(path).mode & 0o022))
      throw new Error(`${filename} is writable after signing: ${path}`);

  }

  process.stdout.write(
    `${JSON.stringify({
      event: 'signed-app',
      app,
      target,
      corednsSha256: digest(join(resources, `coredns-${target}`)),
      caddySha256: digest(join(resources, `caddy-${target}`)),
      status: 'passed',
    })}\n`,
  );
}

function checkSigning(app: string): void {
  if (!process.env.APPLE_SIGNING_IDENTITY)
    throw new Error('APPLE_SIGNING_IDENTITY is required for the release check');

  run('/usr/bin/codesign', ['--verify', '--deep', '--strict', '--verbose=2', app]);
  run('/usr/bin/spctl', ['--assess', '--type', 'execute', '--verbose=4', app]);
  const display = run('/usr/bin/codesign', ['--display', '--verbose=4', app]);

  process.stdout.write(
    `${JSON.stringify({ event: 'codesign', app, display, status: 'passed' })}\n`,
  );
}

function buildPackages(): void {
  const manifestPath = join(process.cwd(), 'sidecars', 'manifest.json');
  const manifest = parseManifest(manifestPath);

  for (const architecture of ARCHITECTURES) {
    const directory = join(process.cwd(), 'sidecars', 'build', architecture.target);

    verifyVersion(
      manifest,
      architecture.name,
      join(directory, `coredns-${architecture.target}`),
      'coredns',
    );
    verifyVersion(
      manifest,
      architecture.name,
      join(directory, `caddy-${architecture.target}`),
      'caddy',
    );
    run('cargo', ['build', '--release', '--target', architecture.target, '-p', 'own-domains-daemon']);
    run('pnpm', [
      'exec',
      'tsx',
      'scripts/stage-desktop.ts',
      '--release',
      '--target',
      architecture.target,
    ]);
    run(
      'pnpm',
      ['exec', 'tauri', 'build', '--target', architecture.target, '--bundles', 'app'],
      join(process.cwd(), 'crates', 'own-domains-desktop'),
    );
    const app = join(
      process.cwd(),
      'target',
      architecture.target,
      'release',
      'bundle',
      'macos',
      'Own Domains.app',
    );

    verifyApp(app, architecture.target, manifestPath);
    checkSigning(app);
  }
}

async function main(): Promise<number> {
  const preflight = process.argv.includes('--preflight');

  await verifyPreflight();
  if (preflight) return 0;

  if ('darwin' !== process.platform)
    throw new Error('package release check requires macOS; use --preflight for hash verification');

  buildPackages();

  return 0;
}

main()
  .then((code) => {
    process.exitCode = code;
  })
  .catch((error: unknown) => {
    process.stderr.write(
      `release check failed: ${error instanceof Error ? error.message : String(error)}\n`,
    );
    process.exitCode = 1;
  });
