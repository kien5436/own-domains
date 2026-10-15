import { request } from 'node:http';

export function applyCaddyConfig(config: object, socketPath = '/tmp/own-domains-caddy.sock', timeoutMs = 3_000): Promise<void> {
  const body = JSON.stringify(config);

  return new Promise((resolve, reject) => {
    const outgoing = request({ socketPath, path: '/load', method: 'POST', headers: { 'content-type': 'application/json', 'content-length': Buffer.byteLength(body) } }, (response) => {
      const chunks: Buffer[] = [];

      response.on('data', (chunk) => chunks.push(Buffer.from(chunk)));
      response.on('end', () => {
        if (response.statusCode && 200 <= response.statusCode && 300 > response.statusCode) resolve();
        else reject(new Error(`Caddy rejected configuration (${response.statusCode}): ${Buffer.concat(chunks).toString('utf8')}`));
      });
    });

    outgoing.on('error', reject);
    outgoing.setTimeout(timeoutMs, () => outgoing.destroy(new Error(`Caddy did not respond within ${timeoutMs}ms`)));
    outgoing.end(body);
  });
}
