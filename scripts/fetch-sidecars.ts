import { spawnSync } from 'node:child_process';
import { createHash, createVerify, X509Certificate } from 'node:crypto';
import {
  chmodSync,
  closeSync,
  existsSync,
  mkdirSync,
  openSync,
  readFileSync,
  renameSync,
  statSync,
  unlinkSync,
  writeSync,
} from 'node:fs';
import { basename, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { gunzipSync } from 'node:zlib';

export type Architecture = 'arm64' | 'amd64';

export type SidecarName = 'coredns' | 'caddy';

export interface HttpResponse {
  status: number;
  headers: Record<string, string | undefined>;
  body: Buffer;
}

export interface Checksum {
  algorithm: 'sha256' | 'sha512';
  url: string;
  value: string;
}

export interface SidecarRelease {
  target: string;
  url: string;
  archiveMember: string;
  checksum: Checksum;
  executableSha256: string;
}

export interface SignatureConfig {
  checksumUrl: string;
  signatureUrl: string;
  certificateUrl: string;
  certificateIdentity: string;
}

export interface SidecarDefinition {
  version: string;
  signature?: SignatureConfig;
  releases: Record<Architecture, SidecarRelease>;
}

export interface SidecarBuildManifest {
  schemaVersion: 1;
  coredns: SidecarDefinition;
  caddy: SidecarDefinition;
}

export interface RuntimeSidecarManifest {
  schemaVersion: 1;
  coredns: { version: string; sha256: string };
  caddy: { version: string; sha256: string };
}

export interface FetchedSidecar {
  architecture: Architecture;
  target: string;
  directory: string;
  corednsPath: string;
  caddyPath: string;
  runtimeManifestPath: string;
}

export interface FetchSidecarsOptions {
  manifest: SidecarBuildManifest;
  outputDirectory: string;
  architectures: Architecture[];
  fetchBytes: (url: string) => Promise<Buffer>;
  verifySignatures?: boolean;
  runVersion?: (path: string, component: SidecarName) => string;
}

export interface VerifySidecarsOptions {
  manifest: SidecarBuildManifest;
  outputDirectory: string;
  architectures: Architecture[];
  runVersion?: (path: string, component: SidecarName) => string;
}

const TARGETS: Record<Architecture, string> = {
  arm64: 'aarch64-apple-darwin',
  amd64: 'x86_64-apple-darwin',
};

const ARCHITECTURES: Architecture[] = ['arm64', 'amd64'];
const MAX_ARCHIVE_BYTES = 256 * 1024 * 1024;
const MAX_EXECUTABLE_BYTES = 128 * 1024 * 1024;
const MAX_TAR_ENTRIES = 4096;
const REDIRECT_LIMIT = 5;
const GITHUB_HOSTS = new Set(['github.com', 'release-assets.githubusercontent.com', 'objects.githubusercontent.com']);

export function parseArchitectures(value: string): Architecture[] {
  if ('all' === value) return [...ARCHITECTURES];
  if ('arm64' === value || 'amd64' === value) return [value];
  throw new Error(`unsupported architecture: ${value}`);
}

function hostArchitecture(): Architecture {
  if ('arm64' === process.arch) return 'arm64';
  if ('x64' === process.arch) return 'amd64';
  throw new Error(`unsupported host architecture: ${process.arch}`);
}

function defaultArchitectures(): Architecture[] {
  return 'darwin' === process.platform ? [hostArchitecture()] : [...ARCHITECTURES];
}

function validateReleaseUrl(url: string): URL {
  const parsed = new URL(url);

  if (
    'https:' !== parsed.protocol
    || 'github.com' !== parsed.hostname
    || parsed.username
    || parsed.password
    || parsed.port
    || !/^\/[^/]+\/[^/]+\/releases\/download\//.test(parsed.pathname)
  )
    throw new Error(`release URL must be an exact GitHub release asset: ${url}`);

  return parsed;
}

function defaultFetchBytes(url: string): Promise<Buffer> {
  return fetchBytesWithRedirects(url, async (requested) => {
    const response = await fetch(requested, { redirect: 'manual' });

    return {
      status: response.status,
      headers: { location: response.headers.get('location') ?? undefined },
      body: Buffer.from(await response.arrayBuffer()),
    };
  });
}

export async function fetchBytesWithRedirects(
  url: string,
  fetcher: (url: string) => Promise<HttpResponse>,
): Promise<Buffer> {
  let current = url;

  for (let attempt = 0; attempt <= REDIRECT_LIMIT; attempt += 1) {
    // Redirects must be followed in order to preserve the bounded chain.
    // eslint-disable-next-line no-await-in-loop
    const response = await fetcher(current);

    if (300 <= response.status && 400 > response.status) {
      const location = response.headers.location;

      if (!location) throw new Error(`redirect from ${current} has no location`);
      const next = new URL(location, current);

      if (!GITHUB_HOSTS.has(next.hostname))
        throw new Error(`refusing redirect outside GitHub release hosts: ${next.hostname}`);

      current = next.toString();

      continue;
    }

    if (200 !== response.status)
      throw new Error(`download failed with HTTP ${response.status}: ${current}`);

    return response.body;
  }

  throw new Error(`too many redirects while downloading ${url}`);
}

function digest(algorithm: 'sha256' | 'sha512', bytes: Buffer): string {
  return createHash(algorithm).update(bytes).digest('hex');
}

function requireHex(value: string, algorithm: 'sha256' | 'sha512', label: string): string {
  const length = 'sha256' === algorithm ? 64 : 128;

  if (!new RegExp(`^[a-f0-9]{${length}}$`).test(value))
    throw new Error(`${label} must be a lowercase ${algorithm} digest`);

  return value;
}

function parseManifest(raw: unknown): SidecarBuildManifest {
  const manifest = raw as SidecarBuildManifest;

  if (1 !== manifest?.schemaVersion) throw new Error('sidecar manifest schemaVersion must be 1');

  for (const name of ['coredns', 'caddy'] as const) {
    const sidecar = manifest[name];

    if (!sidecar || 'string' !== typeof sidecar.version || !sidecar.version)
      throw new Error(`${name} version is required`);

    for (const architecture of ARCHITECTURES) {
      const release = sidecar.releases?.[architecture];

      if (!release) throw new Error(`${name} ${architecture} release is required`);

      if (release.target !== TARGETS[architecture])
        throw new Error(`${name} ${architecture} target must be ${TARGETS[architecture]}`);

      validateReleaseUrl(release.url);
      validateReleaseUrl(release.checksum.url);
      requireHex(release.checksum.value, release.checksum.algorithm, `${name} ${architecture} archive digest`);
      requireHex(release.executableSha256, 'sha256', `${name} ${architecture} executable digest`);

      if (!release.archiveMember || basename(release.archiveMember) !== release.archiveMember)
        throw new Error(`${name} ${architecture} archiveMember must be a plain member name`);

    }

    if (sidecar.signature) {
      validateReleaseUrl(sidecar.signature.checksumUrl);
      validateReleaseUrl(sidecar.signature.signatureUrl);
      validateReleaseUrl(sidecar.signature.certificateUrl);

      if (!sidecar.signature.certificateIdentity.startsWith('https://github.com/'))
        throw new Error(`${name} signature certificate identity must be a GitHub workflow URI`);

      for (const release of Object.values(sidecar.releases))
        if (release.checksum.url !== sidecar.signature.checksumUrl)
          throw new Error(`${name} signature checksum URL must match every release checksum URL`);

    }
  }

  return manifest;
}

function parseChecksumFile(contents: Buffer, filename: string, algorithm: 'sha256' | 'sha512'): string {
  const length = 'sha256' === algorithm ? 64 : 128;

  for (const line of contents.toString('utf8').split(/\r?\n/)) {
    const match = /^([a-fA-F0-9]+)\s+\*?(.+)$/.exec(line.trim());

    if (!match) continue;
    if (basename(match[2]) !== filename) continue;

    if (match[1].length !== length)
      throw new Error(`checksum for ${filename} is not a valid ${algorithm} digest`);

    return match[1].toLowerCase();
  }

  throw new Error(`checksum asset does not contain ${filename}`);
}

function decodeWrapped(value: Buffer): Buffer {
  const text = value.toString('utf8').trim();

  if (text.includes('-----BEGIN')) return Buffer.from(text, 'utf8');
  if (/^[A-Za-z0-9+/=\s]+$/.test(text)) return Buffer.from(text, 'base64');

  return value;
}

function decodeSignature(value: Buffer): Buffer {
  const text = value.toString('utf8').trim();

  if (/^[A-Za-z0-9+/=\s]+$/.test(text) && 0 === text.length % 4)
    return Buffer.from(text, 'base64');

  return value;
}

export interface SignatureVerification {
  checksum: Buffer;
  signature: Buffer;
  certificate: Buffer;
  identity: string;
}

export function verifyChecksumSignature(input: SignatureVerification): void {
  const certificate = new X509Certificate(decodeWrapped(input.certificate));
  const subjectAlternativeName = certificate.subjectAltName ?? '';

  if (!subjectAlternativeName.split(/,\s*/).includes(`URI:${input.identity}`))
    throw new Error('checksum certificate identity does not match the pinned GitHub workflow');

  const verifier = createVerify('sha256');

  verifier.update(input.checksum);
  verifier.end();

  if (!verifier.verify(certificate.publicKey, decodeSignature(input.signature)))
    throw new Error('checksum signature verification failed');

}

function readTarString(header: Buffer, offset: number, length: number): string {
  const end = header.indexOf(0, offset);

  return header.toString('utf8', offset, -1 === end || end > offset + length ? offset + length : end);
}

function readTarSize(header: Buffer): number {
  const value = readTarString(header, 124, 12).trim();

  if (!/^[0-7]+$/.test(value)) throw new Error('archive contains an invalid file size');

  return Number.parseInt(value, 8);
}

export function extractArchiveMember(archive: Buffer, member: string): Buffer {
  let tar: Buffer;

  try {
    tar = gunzipSync(archive, { maxOutputLength: MAX_ARCHIVE_BYTES });
  }
  catch (error) {
    throw new Error(`archive is not a valid gzip file: ${String(error)}`, { cause: error });
  }

  let offset = 0;
  let entries = 0;

  while (offset + 512 <= tar.length) {
    const header = tar.subarray(offset, offset + 512);

    if (header.every((byte) => 0 === byte)) break;
    entries += 1;
    if (entries > MAX_TAR_ENTRIES) throw new Error('archive contains too many entries');
    const name = readTarString(header, 0, 100);
    const prefix = readTarString(header, 345, 155);
    const fullName = prefix ? `${prefix}/${name}` : name;

    if (fullName.startsWith('/') || fullName.split('/').includes('..'))
      throw new Error(`archive contains an unsafe member path: ${fullName}`);

    const size = readTarSize(header);
    const type = header.toString('ascii', 156, 157);
    const contentStart = offset + 512;
    const contentEnd = contentStart + size;

    if (contentEnd > tar.length) throw new Error(`archive member ${fullName} is truncated`);

    if (fullName === member) {
      if ('0' !== type && '\0' !== type) throw new Error(`archive member ${member} is not a regular file`);
      if (size > MAX_EXECUTABLE_BYTES) throw new Error(`archive member ${member} exceeds the executable size limit`);

      return tar.subarray(contentStart, contentEnd);
    }

    offset = contentStart + Math.ceil(size / 512) * 512;
  }

  throw new Error(`archive does not contain the expected ${member} executable`);
}

function atomicWrite(path: string, contents: Buffer, mode: number): void {
  const temporary = `${path}.tmp-${process.pid}-${Date.now()}`;
  const descriptor = openSync(temporary, 'w', mode);

  try {
    writeSync(descriptor, contents);
  }
  finally {
    closeSync(descriptor);
  }

  chmodSync(temporary, mode);

  try {
    renameSync(temporary, path);
  }
  catch (error) {
    unlinkSync(temporary);
    throw error;
  }
}

function binaryPath(directory: string, component: SidecarName, target: string): string {
  return join(directory, `${component}-${target}`);
}

function sidecarExpectation(manifest: SidecarBuildManifest, architecture: Architecture) {
  return {
    target: TARGETS[architecture],
    coredns: manifest.coredns.releases[architecture],
    caddy: manifest.caddy.releases[architecture],
  };
}

async function downloadSidecar(
  name: SidecarName,
  definition: SidecarDefinition,
  release: SidecarRelease,
  fetchBytes: (url: string) => Promise<Buffer>,
  verifySignatures: boolean,
): Promise<Buffer> {
  const fetchOnce = (() => {
    const cache = new Map<string, Promise<Buffer>>();

    return (url: string) => {
      let pending = cache.get(url);

      if (!pending) {
        pending = fetchBytes(url);
        cache.set(url, pending);
      }

      return pending;
    };
  })();
  const checksumBytes = await fetchOnce(release.checksum.url);

  if (definition.signature && verifySignatures) {
    const [signature, certificate] = await Promise.all([
      fetchOnce(definition.signature.signatureUrl),
      fetchOnce(definition.signature.certificateUrl),
    ]);

    verifyChecksumSignature({
      checksum: checksumBytes,
      signature,
      certificate,
      identity: definition.signature.certificateIdentity,
    });
  }

  const archive = await fetchOnce(release.url);

  if (archive.length > MAX_ARCHIVE_BYTES) throw new Error(`${name} archive exceeds the size limit`);
  const expected = requireHex(
    release.checksum.value,
    release.checksum.algorithm,
    `${name} archive digest`,
  );
  const filename = basename(new URL(release.url).pathname);
  const published = parseChecksumFile(checksumBytes, filename, release.checksum.algorithm);

  if (published !== expected)
    throw new Error(`${name} checksum asset does not match the pinned manifest digest`);

  const actual = digest(release.checksum.algorithm, archive);

  if (actual !== expected)
    throw new Error(`${name} archive checksum mismatch: expected ${expected}, received ${actual}`);

  const executable = extractArchiveMember(archive, release.archiveMember);
  const executableSha256 = digest('sha256', executable);

  if (executableSha256 !== release.executableSha256)
    throw new Error(`${name} executable digest mismatch`);

  return executable;
}

export async function fetchSidecars(options: FetchSidecarsOptions): Promise<FetchedSidecar[]> {
  const manifest = parseManifest(options.manifest);
  const fetched: FetchedSidecar[] = [];

  for (const architecture of options.architectures) {
    const { target, coredns, caddy } = sidecarExpectation(manifest, architecture);
    // Downloads are sequential to bound memory and temporary disk use.
    // eslint-disable-next-line no-await-in-loop
    const corednsBytes = await downloadSidecar(
      'coredns',
      manifest.coredns,
      coredns,
      options.fetchBytes,
      options.verifySignatures ?? true,
    );
    // eslint-disable-next-line no-await-in-loop
    const caddyBytes = await downloadSidecar(
      'caddy',
      manifest.caddy,
      caddy,
      options.fetchBytes,
      options.verifySignatures ?? true,
    );
    const directory = join(options.outputDirectory, target);

    mkdirSync(directory, { recursive: true, mode: 0o700 });
    const corednsPath = binaryPath(directory, 'coredns', target);
    const caddyPath = binaryPath(directory, 'caddy', target);

    if (options.runVersion) {
      checkVersion(options.runVersion, corednsPath, 'coredns', manifest.coredns.version, corednsBytes);
      checkVersion(options.runVersion, caddyPath, 'caddy', manifest.caddy.version, caddyBytes);
    }

    atomicWrite(corednsPath, corednsBytes, 0o755);
    atomicWrite(caddyPath, caddyBytes, 0o755);
    const runtimeManifest: RuntimeSidecarManifest = {
      schemaVersion: 1,
      coredns: { version: manifest.coredns.version, sha256: coredns.executableSha256 },
      caddy: { version: manifest.caddy.version, sha256: caddy.executableSha256 },
    };

    atomicWrite(
      join(directory, 'manifest.json'),
      Buffer.from(`${JSON.stringify(runtimeManifest, null, 2)}\n`),
      0o644,
    );
    fetched.push({
      architecture,
      target,
      directory,
      corednsPath,
      caddyPath,
      runtimeManifestPath: join(directory, 'manifest.json'),
    });
  }

  return fetched;
}

function checkVersion(
  runVersion: (path: string, component: SidecarName) => string,
  path: string,
  component: SidecarName,
  expected: string,
  bytes: Buffer,
): void {
  // The caller supplies the already-verified bytes through the temporary path used below.
  const temporary = `${path}.version-${process.pid}-${Date.now()}`;

  atomicWrite(temporary, bytes, 0o755);

  try {
    const output = runVersion(temporary, component);

    if (!output.includes(expected))
      throw new Error(`${component} version mismatch: expected ${expected}, received ${output.trim()}`);

  }
  finally {
    if (existsSync(temporary)) unlinkSync(temporary);
  }
}

export async function verifySidecars(options: VerifySidecarsOptions): Promise<FetchedSidecar[]> {
  await Promise.resolve();
  const manifest = parseManifest(options.manifest);
  const verified: FetchedSidecar[] = [];

  for (const architecture of options.architectures) {
    const { target, coredns, caddy } = sidecarExpectation(manifest, architecture);
    const directory = join(options.outputDirectory, target);
    const runtimeManifestPath = join(directory, 'manifest.json');

    if (!existsSync(runtimeManifestPath)) throw new Error(`runtime sidecar manifest is missing: ${runtimeManifestPath}`);
    const runtime = JSON.parse(readFileSync(runtimeManifestPath, 'utf8')) as RuntimeSidecarManifest;

    if (
      1 !== runtime.schemaVersion
      || runtime.coredns?.version !== manifest.coredns.version
      || runtime.caddy?.version !== manifest.caddy.version
      || runtime.coredns?.sha256 !== coredns.executableSha256
      || runtime.caddy?.sha256 !== caddy.executableSha256
    )
      throw new Error(`runtime sidecar manifest does not match the pinned build manifest: ${runtimeManifestPath}`);

    for (const [component, release] of [
      ['coredns', coredns],
      ['caddy', caddy],
    ] as const) {
      const path = binaryPath(directory, component, target);
      const actual = existsSync(path) ? digest('sha256', readFileSync(path)) : '';

      if (actual !== release.executableSha256)
        throw new Error(`${component} digest mismatch for ${path}`);

      const mode = statSync(path).mode & 0o777;

      if (0 === (mode & 0o111)) throw new Error(`${component} is not executable: ${path}`);

      if (options.runVersion) {
        const output = options.runVersion(path, component);
        const expected = 'coredns' === component ? manifest.coredns.version : manifest.caddy.version;

        if (!output.includes(expected))
          throw new Error(`${component} version mismatch: expected ${expected}, received ${output.trim()}`);

      }
    }

    verified.push({
      architecture,
      target,
      directory,
      corednsPath: binaryPath(directory, 'coredns', target),
      caddyPath: binaryPath(directory, 'caddy', target),
      runtimeManifestPath,
    });
  }

  return verified;
}

function parseArguments(argv: string[]) {
  let verifyOnly = false;
  let architectureValue: string | undefined;
  let outputDirectory = join(process.cwd(), 'sidecars', 'build');
  let manifestPath = join(process.cwd(), 'sidecars', 'manifest.json');

  for (let index = 0; index < argv.length; index += 1)
    switch (argv[index]) {
      case '--':
        break;
      case '--verify-only':
        verifyOnly = true;

        break;
      case '--arch':
        architectureValue = argv[++index];
        if (!architectureValue) throw new Error('--arch requires arm64, amd64, or all');

        break;
      case '--output':
        outputDirectory = resolve(argv[++index] ?? '');
        if (!argv[index]) throw new Error('--output requires a path');

        break;
      case '--manifest':
        manifestPath = resolve(argv[++index] ?? '');
        if (!argv[index]) throw new Error('--manifest requires a path');

        break;
      default:
        throw new Error(`unknown option: ${argv[index]}`);
    }

  return {
    verifyOnly,
    architectures: architectureValue ? parseArchitectures(architectureValue) : defaultArchitectures(),
    outputDirectory,
    manifestPath,
  };
}

function nativeVersion(path: string, component: SidecarName): string {
  const result = spawnSync(path, 'coredns' === component ? ['-version'] : ['version'], {
    encoding: 'utf8',
    timeout: 10_000,
  });

  if (result.error) throw result.error;
  if (0 !== result.status) throw new Error(`${component} version command exited with ${result.status}`);

  return `${result.stdout ?? ''}${result.stderr ?? ''}`;
}

export async function runCli(argv: string[]): Promise<number> {
  try {
    const options = parseArguments(argv);
    const manifest = parseManifest(
      JSON.parse(readFileSync(options.manifestPath, 'utf8')) as SidecarBuildManifest,
    );
    const shouldRunVersions
      = 'darwin' === process.platform
        && options.architectures.every((architecture) => TARGETS[architecture] === TARGETS[hostArchitecture()]);
    const runVersion = shouldRunVersions ? nativeVersion : undefined;
    const sidecars = options.verifyOnly
      ? await verifySidecars({
        manifest,
        outputDirectory: options.outputDirectory,
        architectures: options.architectures,
        runVersion,
      })
      : await fetchSidecars({
        manifest,
        outputDirectory: options.outputDirectory,
        architectures: options.architectures,
        fetchBytes: defaultFetchBytes,
        runVersion,
      });

    for (const sidecar of sidecars)
      process.stdout.write(`verified ${sidecar.target} sidecars in ${sidecar.directory}\n`);

    return 0;
  }
  catch (error) {
    process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);

    return 1;
  }
}

const invokedPath = process.argv[1] ? resolve(process.argv[1]) : '';

if (invokedPath && fileURLToPath(import.meta.url) === invokedPath)
  runCli(process.argv.slice(2)).then((code) => {
    process.exitCode = code;
  });
