import { createServer } from 'node:http';

import { afterEach, describe, expect, it } from 'vitest';

import { applyCaddyConfig, readCaddyRootCertificate } from '../../../src/server/proxy/client';

const closeCallbacks: Array<() => Promise<void>> = [];

afterEach(async () => {
  await Promise.all(closeCallbacks.splice(0).map((close) => close()));
});

describe('applyCaddyConfig', () => {
  it('loads configuration through a loopback TCP admin endpoint', async () => {
    let receivedBody = '';
    const server = createServer((request, response) => {
      request.setEncoding('utf8');
      request.on('data', (chunk) => { receivedBody += chunk });
      request.on('end', () => {
        expect(request.method).toBe('POST');
        expect(request.url).toBe('/load');
        response.writeHead(200).end();
      });
    });

    await new Promise<void>((resolve, reject) => {
      server.once('error', reject);
      server.listen(0, '127.0.0.1', resolve);
    });
    const address = server.address();

    if (!address || 'string' === typeof address) throw new Error('test server did not bind');
    closeCallbacks.push(() => new Promise((resolve, reject) => {
      server.close((error) => {
        if (error) reject(error);
        else resolve();
      });
    }));

    await applyCaddyConfig({ admin: { listen: 'test' } }, `127.0.0.1:${address.port}`);

    expect(JSON.parse(receivedBody)).toEqual({ admin: { listen: 'test' } });
  });

  it('reads only the public root certificate from Caddy PKI', async () => {
    const certificate = '-----BEGIN CERTIFICATE-----\ntest\n-----END CERTIFICATE-----\n';
    const server = createServer((request, response) => {
      expect(request.method).toBe('GET');
      expect(request.url).toBe('/pki/ca/local');
      response.writeHead(200, { 'content-type': 'application/json' }).end(JSON.stringify({ root_certificate: certificate, intermediate_certificate: 'not returned' }));
    });

    await new Promise<void>((resolve, reject) => {
      server.once('error', reject);
      server.listen(0, '127.0.0.1', resolve);
    });
    const address = server.address();

    if (!address || 'string' === typeof address) throw new Error('test server did not bind');
    closeCallbacks.push(() => new Promise((resolve, reject) => {
      server.close((error) => {
        if (error) reject(error);
        else resolve();
      });
    }));

    await expect(readCaddyRootCertificate(`127.0.0.1:${address.port}`)).resolves.toBe(certificate);
  });
});
