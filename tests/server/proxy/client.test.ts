import { createServer } from 'node:http';

import { afterEach, describe, expect, it } from 'vitest';

import { applyCaddyConfig } from '../../../src/server/proxy/client';

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
});
