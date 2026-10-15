import { type ChildProcess, execFile, spawn, spawnSync } from 'node:child_process';
import { createSocket, type Socket } from 'node:dgram';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { createServer } from 'node:net';
import { networkInterfaces, tmpdir } from 'node:os';
import { join } from 'node:path';
import { promisify } from 'node:util';

import { afterEach, describe, expect, it } from 'vitest';

import { buildDnsConfig } from '../../src/server/network/dns-config';
import type { Settings } from '../../src/shared/types';

const dnsmasqAvailable = 0 === spawnSync('dnsmasq', ['--version'], { stdio: 'ignore' }).status;
const digAvailable = 0 === spawnSync('dig', ['-v'], { stdio: 'ignore' }).status;
const processes: ChildProcess[] = [];
const sockets: Socket[] = [];
const directories: string[] = [];
const execFileAsync = promisify(execFile);

afterEach(async () => {
  await Promise.all(processes.splice(0).map((process) => new Promise<void>((resolve) => {
    if (null !== process.exitCode) { resolve() }
    else { process.once('exit', () => resolve()); process.kill('SIGTERM') }
  })));
  await Promise.all(sockets.splice(0).map((socket) => new Promise<void>((resolve) => { socket.close(() => resolve()) })));
  await Promise.all(directories.splice(0).map((directory) => rm(directory, { recursive: true, force: true })));
});

describe.runIf(dnsmasqAvailable && digAvailable)('dnsmasq integration', () => {
  it('answers .own over UDP and TCP, returns no AAAA data, and forwards public names', { timeout: 15_000 }, async () => {
    const upstream = await startUpstreamDns();
    const dnsPort = await availableTcpPort();
    const directory = await mkdtemp(join(tmpdir(), 'own-domains-dns-'));

    directories.push(directory);
    const settings: Settings = { schemaVersion: 1, revision: 1, lanAddress: '192.168.50.10', upstreamDns: [upstream.address], mappings: [] };
    const configPath = join(directory, 'dnsmasq.conf');

    await writeFile(configPath, `${buildDnsConfig(settings, { port: dnsPort, listenAddresses: ['127.0.0.1'], upstreamPort: upstream.port })}pid-file=${join(directory, 'dnsmasq.pid')}\n`);
    const validation = spawnSync('dnsmasq', ['--test', `--conf-file=${configPath}`], { encoding: 'utf8' });

    expect(validation.status, validation.stderr).toBe(0);
    const dnsmasq = spawn('dnsmasq', ['--keep-in-foreground', `--conf-file=${configPath}`], { stdio: ['ignore', 'pipe', 'pipe'] });

    processes.push(dnsmasq);
    await waitForDns(dnsPort, dnsmasq);

    await expect(query(dnsPort, 'shop.own', 'A')).resolves.toBe('192.168.50.10');
    await expect(query(dnsPort, 'api.shop.own', 'A', true)).resolves.toBe('192.168.50.10');
    await expect(query(dnsPort, 'shop.own', 'AAAA')).resolves.toBe('');
    await expect(query(dnsPort, 'public.example', 'A')).resolves.toBe('203.0.113.7');
  });
});

async function startUpstreamDns(): Promise<{ address: string; port: number }> {
  const address = Object.values(networkInterfaces()).flat().find((candidate) => candidate && 'IPv4' === candidate.family && !candidate.internal)?.address;

  if (!address) throw new Error('A non-internal IPv4 address is required for the DNS forwarding fixture');
  const socket = createSocket('udp4');

  sockets.push(socket);
  socket.on('message', (message, remote) => {
    const answer = Buffer.alloc(16);

    answer.writeUInt16BE(0xc00c, 0);
    answer.writeUInt16BE(1, 2);
    answer.writeUInt16BE(1, 4);
    answer.writeUInt32BE(60, 6);
    answer.writeUInt16BE(4, 10);
    Buffer.from([203, 0, 113, 7]).copy(answer, 12);
    const response = Buffer.concat([message.subarray(0, dnsQuestionEnd(message)), answer]);

    response.writeUInt16BE(0x8180, 2);
    response.writeUInt16BE(1, 6);
    response.writeUInt16BE(0, 8);
    response.writeUInt16BE(0, 10);
    socket.send(response, remote.port, remote.address);
  });

  const port = await new Promise<number>((resolve, reject) => {
    socket.once('error', reject);
    socket.bind(0, address, () => resolve((socket.address() as { port: number }).port));
  });

  return { address, port };
}

function dnsQuestionEnd(message: Buffer): number {
  let offset = 12;

  while (0 !== message[offset]) offset += (message[offset] ?? 0) + 1;

  return offset + 5;
}

async function availableTcpPort(): Promise<number> {
  const server = createServer();
  const port = await new Promise<number>((resolve) => { server.listen(0, '127.0.0.1', () => resolve((server.address() as { port: number }).port)) });

  await new Promise<void>((resolve) => { server.close(() => resolve()) });

  return port;
}

async function query(port: number, hostname: string, type: 'A' | 'AAAA', tcp = false): Promise<string> {
  const result = await execFileAsync('dig', ['@127.0.0.1', '-p', String(port), hostname, type, '+short', ...(tcp ? ['+tcp'] : [])], { encoding: 'utf8', timeout: 2_000 });

  return result.stdout.trim();
}

async function waitForDns(port: number, process: ChildProcess, attempt = 0): Promise<void> {
  if (null !== process.exitCode) throw new Error(`dnsmasq exited before becoming ready (${process.exitCode})`);
  if (50 <= attempt) throw new Error('dnsmasq did not become ready');

  try {
    await query(port, 'ready.own', 'A');
  }
  catch {
    await new Promise((resolve) => { setTimeout(resolve, 20) });
    await waitForDns(port, process, attempt + 1);
  }
}
