import { request } from 'node:http';

export function applyCaddyConfig(config: object, adminAddress = '127.0.0.1:2020', timeoutMs = 3_000): Promise<void> {
  const body = JSON.stringify(config);
  const endpoint = adminAddress.startsWith('/') ? { socketPath: adminAddress } : tcpEndpoint(adminAddress);

  return new Promise((resolve, reject) => {
    const outgoing = request({ ...endpoint, path: '/load', method: 'POST', headers: { 'content-type': 'application/json', 'content-length': Buffer.byteLength(body) } }, (response) => {
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

export function readCaddyRootCertificate(adminAddress = '127.0.0.1:2020', timeoutMs = 3_000): Promise<string> {
  const endpoint = adminAddress.startsWith('/') ? { socketPath: adminAddress } : tcpEndpoint(adminAddress);

  return new Promise((resolve, reject) => {
    const outgoing = request({ ...endpoint, path: '/pki/ca/local', method: 'GET' }, (response) => {
      const chunks: Buffer[] = [];

      response.on('data', (chunk) => chunks.push(Buffer.from(chunk)));
      response.on('end', () => {
        try {
          if (!response.statusCode || 200 > response.statusCode || 300 <= response.statusCode) throw new Error(`Caddy PKI request failed (${response.statusCode})`);
          const payload = JSON.parse(Buffer.concat(chunks).toString('utf8')) as { root_certificate?: unknown };

          if ('string' !== typeof payload.root_certificate || !payload.root_certificate.includes('-----BEGIN CERTIFICATE-----')) throw new Error('Caddy PKI response did not include a root certificate');
          resolve(payload.root_certificate);
        }
        catch (error) { reject(error) }
      });
    });

    outgoing.on('error', reject);
    outgoing.setTimeout(timeoutMs, () => outgoing.destroy(new Error(`Caddy did not respond within ${timeoutMs}ms`)));
    outgoing.end();
  });
}

function tcpEndpoint(adminAddress: string): { hostname: string; port: number } {
  const separator = adminAddress.lastIndexOf(':');
  const hostname = adminAddress.slice(0, separator);
  const port = Number(adminAddress.slice(separator + 1));

  if (!hostname || !Number.isInteger(port) || 0 >= port || 65_535 < port) throw new Error(`Invalid Caddy admin address: ${adminAddress}`);

  return { hostname, port };
}
