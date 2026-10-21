import { createHash } from 'node:crypto';
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { gzipSync } from 'node:zlib';

import { afterEach, describe, expect, it } from 'vitest';

import {
  type Architecture,
  fetchBytesWithRedirects,
  fetchSidecars,
  type HttpResponse,
  parseArchitectures,
  type SidecarBuildManifest,
  verifyChecksumSignature,
  verifySidecars,
} from '../../scripts/fetch-sidecars';

const directories: string[] = [];

function temporaryDirectory(): string {
  const directory = mkdtempSync(join(tmpdir(), 'own-domains-sidecars-'));

  directories.push(directory);

  return directory;
}

afterEach(() => {
  for (const directory of directories.splice(0))
    rmSync(directory, { force: true, recursive: true });

});

function writeOctal(buffer: Buffer, value: number, offset: number, length: number): void {
  buffer.write(`${value.toString(8).padStart(length - 1, '0')}\0`, offset, length, 'ascii');
}

function tarGz(entries: { name: string; content: string }[]): Buffer {
  const chunks: Buffer[] = [];

  for (const entry of entries) {
    const contents = Buffer.from(entry.content);
    const header = Buffer.alloc(512);

    header.write(entry.name, 0, 100, 'utf8');
    writeOctal(header, 0o755, 100, 8);
    writeOctal(header, 0, 108, 8);
    writeOctal(header, 0, 116, 8);
    writeOctal(header, contents.length, 124, 12);
    writeOctal(header, 0, 136, 12);
    header.fill(0x20, 148, 156);
    header.write('0', 156, 1, 'ascii');
    header.write('ustar\0', 257, 6, 'ascii');
    header.write('00', 263, 2, 'ascii');
    let checksum = 0;

    for (const byte of header) checksum += byte;
    header.write(`${checksum.toString(8).padStart(6, '0')}\0 `, 148, 8, 'ascii');
    chunks.push(header, contents, Buffer.alloc((512 - (contents.length % 512)) % 512));
  }

  chunks.push(Buffer.alloc(1024));

  return gzipSync(Buffer.concat(chunks));
}

function digest(algorithm: 'sha256' | 'sha512', bytes: Buffer): string {
  return createHash(algorithm).update(bytes).digest('hex');
}

interface FixtureOptions {
  coreArchive?: Buffer;
  caddyArchive?: Buffer;
  coreDigestOverride?: string;
  caddyDigestOverride?: string;
  coreMember?: string;
}

function fixture(options: FixtureOptions = {}) {
  const coreArchive = options.coreArchive ?? tarGz([{ name: options.coreMember ?? 'coredns', content: 'core-binary' }]);
  const caddyArchive = options.caddyArchive ?? tarGz([{ name: 'caddy', content: 'caddy-binary' }]);
  const coreSha = digest('sha256', coreArchive);
  const caddySha = digest('sha512', caddyArchive);
  const coreChecksum = Buffer.from(`${coreSha}  core-arm64.tgz\n`);
  const caddyChecksum = Buffer.from(`${caddySha}  caddy-arm64.tar.gz\n`);
  const manifest: SidecarBuildManifest = {
    schemaVersion: 1,
    coredns: {
      version: '1.0.0',
      releases: {
        arm64: {
          target: 'aarch64-apple-darwin',
          url: 'https://github.com/example/sidecars/releases/download/v1/core-arm64.tgz',
          archiveMember: 'coredns',
          checksum: {
            algorithm: 'sha256',
            url: 'https://github.com/example/sidecars/releases/download/v1/core-arm64.tgz.sha256',
            value: options.coreDigestOverride ?? coreSha,
          },
          executableSha256: digest('sha256', Buffer.from('core-binary')),
        },
        amd64: {
          target: 'x86_64-apple-darwin',
          url: 'https://github.com/example/sidecars/releases/download/v1/core-amd64.tgz',
          archiveMember: 'coredns',
          checksum: {
            algorithm: 'sha256',
            url: 'https://github.com/example/sidecars/releases/download/v1/core-amd64.tgz.sha256',
            value: coreSha,
          },
          executableSha256: digest('sha256', Buffer.from('core-binary')),
        },
      },
    },
    caddy: {
      version: '1.0.0',
      releases: {
        arm64: {
          target: 'aarch64-apple-darwin',
          url: 'https://github.com/example/sidecars/releases/download/v1/caddy-arm64.tar.gz',
          archiveMember: 'caddy',
          checksum: {
            algorithm: 'sha512',
            url: 'https://github.com/example/sidecars/releases/download/v1/checksums.txt',
            value: options.caddyDigestOverride ?? caddySha,
          },
          executableSha256: digest('sha256', Buffer.from('caddy-binary')),
        },
        amd64: {
          target: 'x86_64-apple-darwin',
          url: 'https://github.com/example/sidecars/releases/download/v1/caddy-amd64.tar.gz',
          archiveMember: 'caddy',
          checksum: {
            algorithm: 'sha512',
            url: 'https://github.com/example/sidecars/releases/download/v1/checksums.txt',
            value: caddySha,
          },
          executableSha256: digest('sha256', Buffer.from('caddy-binary')),
        },
      },
    },
  };
  const responses = new Map<string, HttpResponse>([
    [manifest.coredns.releases.arm64.url, { status: 200, headers: {}, body: coreArchive }],
    [manifest.coredns.releases.arm64.checksum.url, { status: 200, headers: {}, body: coreChecksum }],
    [manifest.caddy.releases.arm64.url, { status: 200, headers: {}, body: caddyArchive }],
    [manifest.caddy.releases.arm64.checksum.url, { status: 200, headers: {}, body: caddyChecksum }],
  ]);

  return { manifest, responses };
}

function fetcher(responses: Map<string, HttpResponse>) {
  return (url: string): Promise<Buffer> => {
    const response = responses.get(url);

    if (!response) return Promise.reject(new Error(`unexpected URL ${url}`));

    return Promise.resolve(response.body);
  };
}

function versionOutput(component: 'coredns' | 'caddy'): string {
  return 'coredns' === component ? 'CoreDNS-1.0.0' : 'v1.0.0';
}

describe('sidecar downloader', () => {
  it('fetches verified sidecars, writes executable modes, and records a per-target runtime manifest', async () => {
    const { manifest, responses } = fixture();
    const outputDirectory = temporaryDirectory();

    const fetched = await fetchSidecars({
      manifest,
      outputDirectory,
      architectures: ['arm64'],
      fetchBytes: fetcher(responses),
      runVersion: (path, component) => versionOutput(component),
    });

    expect(fetched).toHaveLength(1);
    const target = join(outputDirectory, 'aarch64-apple-darwin');

    expect(readFileSync(join(target, 'coredns-aarch64-apple-darwin'), 'utf8')).toBe('core-binary');
    expect(readFileSync(join(target, 'caddy-aarch64-apple-darwin'), 'utf8')).toBe('caddy-binary');
    expect(statSync(join(target, 'coredns-aarch64-apple-darwin')).mode & 0o777).toBe(0o755);
    expect(statSync(join(target, 'caddy-aarch64-apple-darwin')).mode & 0o777).toBe(0o755);
    expect(JSON.parse(readFileSync(join(target, 'manifest.json'), 'utf8'))).toEqual({
      schemaVersion: 1,
      coredns: { version: '1.0.0', sha256: digest('sha256', Buffer.from('core-binary')) },
      caddy: { version: '1.0.0', sha256: digest('sha256', Buffer.from('caddy-binary')) },
    });
    expect(existsSync(join(target, 'coredns-aarch64-apple-darwin.tmp'))).toBe(false);
  });

  it('rejects an archive whose digest does not match the pinned manifest', async () => {
    const { manifest, responses } = fixture({ coreDigestOverride: '0'.repeat(64) });

    await expect(
      fetchSidecars({
        manifest,
        outputDirectory: temporaryDirectory(),
        architectures: ['arm64'],
        fetchBytes: fetcher(responses),
      }),
    ).rejects.toThrow(/checksum/i);
  });

  it('rejects an archive without the expected executable member', async () => {
    const { manifest, responses } = fixture({ coreMember: 'not-coredns' });

    await expect(
      fetchSidecars({
        manifest,
        outputDirectory: temporaryDirectory(),
        architectures: ['arm64'],
        fetchBytes: fetcher(responses),
      }),
    ).rejects.toThrow(/coredns/);
  });

  it('rejects a binary whose reported version differs from the manifest', async () => {
    const { manifest, responses } = fixture();
    const outputDirectory = temporaryDirectory();

    await expect(
      fetchSidecars({
        manifest,
        outputDirectory,
        architectures: ['arm64'],
        fetchBytes: fetcher(responses),
        runVersion: () => 'CoreDNS-0.9.9',
      }),
    ).rejects.toThrow(/version/i);
    expect(existsSync(join(outputDirectory, 'aarch64-apple-darwin', 'coredns-aarch64-apple-darwin'))).toBe(false);
  });

  it('rejects a partial download without replacing an existing cache entry', async () => {
    const { manifest, responses } = fixture();
    const outputDirectory = temporaryDirectory();
    const target = join(outputDirectory, 'aarch64-apple-darwin');

    mkdirSync(target, { recursive: true });
    const existing = join(target, 'coredns-aarch64-apple-darwin');

    writeFileSync(existing, 'old-binary');
    const archive = responses.get(manifest.coredns.releases.arm64.url)!.body;

    responses.set(manifest.coredns.releases.arm64.url, {
      status: 200,
      headers: {},
      body: archive.subarray(0, archive.length - 10),
    });

    await expect(
      fetchSidecars({
        manifest,
        outputDirectory,
        architectures: ['arm64'],
        fetchBytes: fetcher(responses),
      }),
    ).rejects.toThrow(/checksum/i);
    expect(readFileSync(existing, 'utf8')).toBe('old-binary');
  });

  it('rejects unsupported architectures and accepts all', () => {
    expect(() => parseArchitectures('universal')).toThrow(/unsupported/i);
    expect(parseArchitectures('all')).toEqual(['arm64', 'amd64']);
    expect(parseArchitectures('arm64')).toEqual(['arm64']);
  });

  it('rejects redirects outside the GitHub release hosts', async () => {
    const responses = new Map<string, HttpResponse>([
      [
        'https://github.com/example/sidecars/releases/download/v1/core.tgz',
        { status: 302, headers: { location: 'https://example.com/archive.tgz' }, body: Buffer.alloc(0) },
      ],
    ]);

    await expect(
      fetchBytesWithRedirects(
        'https://github.com/example/sidecars/releases/download/v1/core.tgz',
        (url) => Promise.resolve(responses.get(url) ?? { status: 404, headers: {}, body: Buffer.alloc(0) }),
      ),
    ).rejects.toThrow(/redirect/i);
  });

  it('follows an allowed GitHub asset redirect', async () => {
    const responses = new Map<string, HttpResponse>([
      [
        'https://github.com/example/sidecars/releases/download/v1/core.tgz',
        {
          status: 302,
          headers: { location: 'https://release-assets.githubusercontent.com/github-production-release-asset/1' },
          body: Buffer.alloc(0),
        },
      ],
      [
        'https://release-assets.githubusercontent.com/github-production-release-asset/1',
        { status: 200, headers: {}, body: Buffer.from('archive') },
      ],
    ]);

    await expect(
      fetchBytesWithRedirects(
        'https://github.com/example/sidecars/releases/download/v1/core.tgz',
        (url) => Promise.resolve(responses.get(url) ?? { status: 404, headers: {}, body: Buffer.alloc(0) }),
      ),
    ).resolves.toEqual(Buffer.from('archive'));
  });

  it('verifies the published checksum signature and certificate identity', () => {
    const directory = join(process.cwd(), 'tests', 'fixtures', 'sidecars');
    const checksum = readFileSync(join(directory, 'checksums.txt'));
    const signature = readFileSync(join(directory, 'certificate.sig'));
    const certificate = readFileSync(join(directory, 'certificate.pem'));
    const identity = 'https://github.com/example/sidecars/.github/workflows/release.yml@refs/tags/v1.0.0';

    expect(() => verifyChecksumSignature({ checksum, signature, certificate, identity })).not.toThrow();
    expect(() =>
      verifyChecksumSignature({
        checksum: Buffer.concat([checksum, Buffer.from('tampered')]),
        signature,
        certificate,
        identity,
      }),
    ).toThrow(/signature/i);
  });

  it('verifies fetched sidecars without downloading and detects mutation', async () => {
    const { manifest, responses } = fixture();
    const outputDirectory = temporaryDirectory();

    await fetchSidecars({
      manifest,
      outputDirectory,
      architectures: ['arm64'],
      fetchBytes: fetcher(responses),
      runVersion: (path, component) => versionOutput(component),
    });

    await expect(
      verifySidecars({
        manifest,
        outputDirectory,
        architectures: ['arm64'],
        runVersion: (path, component) => versionOutput(component),
      }),
    ).resolves.toHaveLength(1);

    chmodSync(join(outputDirectory, 'aarch64-apple-darwin', 'coredns-aarch64-apple-darwin'), 0o755);
    writeFileSync(join(outputDirectory, 'aarch64-apple-darwin', 'coredns-aarch64-apple-darwin'), 'mutated');
    await expect(
      verifySidecars({
        manifest,
        outputDirectory,
        architectures: ['arm64'] as Architecture[],
      }),
    ).rejects.toThrow(/digest|hash/i);
  });
});
