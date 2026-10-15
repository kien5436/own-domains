import { type ChildProcess, spawn, spawnSync } from 'node:child_process';
import { access, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { createServer, request, type Server } from 'node:http';
import { connect } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';

import { applyCaddyConfig } from '../../src/server/proxy/client';
import { buildProxyConfig } from '../../src/server/proxy/config';
import type { Settings } from '../../src/shared/types';

const caddyAvailable = 0 === spawnSync('caddy', ['version'], { stdio: 'ignore' }).status;
const resources: Array<Server | ChildProcess> = [];
const tempDirectories: string[] = [];

afterEach(async () => {
  await Promise.all(resources.splice(0).map((resource) => 'close' in resource
    ? new Promise<void>((resolve) => { resource.close(() => resolve()) })
    : new Promise<void>((resolve) => { resource.once('exit', () => resolve()); resource.kill('SIGTERM') })));
  await Promise.all(tempDirectories.splice(0).map((directory) => rm(directory, { recursive: true, force: true })));
});

describe.runIf(caddyAvailable)('Caddy integration', () => {
  it('proxies HTTP, streaming, and WebSockets while preserving routes after a rejected reload', { timeout: 15_000 }, async () => {
    const upstream = createServer((incoming, response) => {
      response.writeHead(200, { 'content-type': 'text/plain' });
      response.write(`${incoming.method} ${incoming.url}\n`);
      setTimeout(() => response.end('stream-complete'), 20);
    });

    upstream.on('upgrade', (incoming, socket) => {
      void incoming;
      socket.write('HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\nwebsocket-ready');
      socket.end();
    });
    resources.push(upstream);
    const upstreamPort = await listen(upstream);
    const httpPort = await availablePort();
    const httpsPort = await availablePort();
    const setupPort = await availablePort();
    const directory = await mkdtemp(join(tmpdir(), 'own-domains-caddy-'));

    tempDirectories.push(directory);
    const socketPath = join(directory, 'admin.sock');
    const configPath = join(directory, 'initial.json');

    await writeFile(configPath, JSON.stringify({ admin: { listen: `unix/${socketPath}` } }));
    const caddy = spawn('caddy', ['run', '--config', configPath], { env: { ...process.env, XDG_CONFIG_HOME: directory, XDG_DATA_HOME: directory }, stdio: ['ignore', 'pipe', 'pipe'] });
    let caddyErrors = '';

    caddy.stderr?.on('data', (chunk) => { caddyErrors += String(chunk) });

    resources.push(caddy);
    await waitForAdmin(socketPath, caddy, () => caddyErrors);
    const settings: Settings = {
      schemaVersion: 1,
      revision: 1,
      lanAddress: '127.0.0.1',
      upstreamDns: ['1.1.1.1'],
      mappings: [{ id: 'shop', hostname: 'shop.own', upstreamPort, tls: false, enabled: true }],
    };

    await applyCaddyConfig(buildProxyConfig(settings, { adminListen: `unix/${socketPath}`, addresses: ['127.0.0.1'], httpPort, httpsPort, setupPort }), socketPath);
    await expect(httpGet(httpPort, 'shop.own', '/items?q=1')).resolves.toEqual({ status: 200, body: 'GET /items?q=1\nstream-complete' });
    await expect(httpGet(httpPort, 'unknown.own', '/')).resolves.toMatchObject({ status: 404 });
    await expect(websocketUpgrade(httpPort, 'shop.own')).resolves.toContain('101 Switching Protocols');
    await expect(applyCaddyConfig({ apps: { http: { servers: { broken: { listen: ['bad-address'] } } } } }, socketPath)).rejects.toThrow(/rejected configuration/i);
    await expect(httpGet(httpPort, 'shop.own', '/still-working')).resolves.toMatchObject({ status: 200, body: 'GET /still-working\nstream-complete' });
  });
});

function listen(server: Server): Promise<number> {
  return new Promise((resolve) => { server.listen(0, '127.0.0.1', () => resolve((server.address() as { port: number }).port)) });
}

async function availablePort(): Promise<number> {
  const server = createServer();
  const port = await listen(server);

  await new Promise<void>((resolve) => { server.close(() => resolve()) });

  return port;
}

function httpGet(port: number, host: string, path: string): Promise<{ status: number; body: string }> {
  return new Promise((resolve, reject) => {
    const outgoing = request({ host: '127.0.0.1', port, path, headers: { host } }, (response) => {
      const chunks: Buffer[] = [];

      response.on('data', (chunk) => chunks.push(Buffer.from(chunk)));
      response.on('end', () => resolve({ status: response.statusCode ?? 0, body: Buffer.concat(chunks).toString() }));
    });

    outgoing.on('error', reject);
    outgoing.end();
  });
}

function websocketUpgrade(port: number, host: string): Promise<string> {
  return new Promise((resolve, reject) => {
    const socket = connect(port, '127.0.0.1', () => socket.write(`GET /socket HTTP/1.1\r\nHost: ${host}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n`));
    const chunks: Buffer[] = [];

    socket.on('data', (chunk) => chunks.push(Buffer.from(chunk)));
    socket.on('end', () => resolve(Buffer.concat(chunks).toString()));
    socket.on('error', reject);
  });
}

async function waitForAdmin(socketPath: string, process: ChildProcess, errors: () => string): Promise<void> {
  await waitForAdminAttempt(socketPath, process, errors, 0);
}

async function waitForAdminAttempt(socketPath: string, process: ChildProcess, errors: () => string, attempt: number): Promise<void> {
  if (null !== process.exitCode) throw new Error(`Caddy exited before its admin socket was ready (${process.exitCode}): ${errors()}`);

  try {
    await access(socketPath);
  }
  catch (error) {
    if (50 <= attempt) throw new Error(`Caddy admin socket was not ready: ${String(error)}\n${errors()}`, { cause: error });
    await new Promise((resolve) => { setTimeout(resolve, 20) });
    await waitForAdminAttempt(socketPath, process, errors, attempt + 1);
  }
}
