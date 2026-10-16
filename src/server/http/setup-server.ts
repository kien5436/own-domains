import { createServer } from 'node:http';

import { buildAppleCaProfile } from './apple-profile';

export async function createSetupServer(options: { host: string; port: number; readCaCertificate: () => Promise<string> }): Promise<{ origin: string; close: () => Promise<void> }> {
  const server = createServer(async (request, response) => {
    response.setHeader('X-Content-Type-Options', 'nosniff');

    if ('/ca.crt' === request.url) {
      try {
        const certificate = await options.readCaCertificate();

        response.writeHead(200, { 'content-type': 'application/x-x509-ca-cert', 'content-disposition': 'attachment; filename="own-domains-ca.crt"' });
        response.end(certificate);
      }
      catch {
        response.writeHead(404, { 'content-type': 'text/plain; charset=utf-8' });
        response.end('The local CA certificate is not available yet. Enable one HTTPS domain first.');
      }

      return;
    }

    if ('/ca.mobileconfig' === request.url) {
      try {
        const profile = buildAppleCaProfile(await options.readCaCertificate());

        response.writeHead(200, { 'content-type': 'application/x-apple-aspen-config', 'content-disposition': 'attachment; filename="own-domains-ca.mobileconfig"' });
        response.end(profile);
      }
      catch {
        response.writeHead(404, { 'content-type': 'text/plain; charset=utf-8' });
        response.end('The local CA profile is not available yet. Enable one HTTPS domain first.');
      }

      return;
    }

    response.writeHead(200, { 'content-type': 'text/html; charset=utf-8', 'cache-control': 'no-store' });
    response.end('<!doctype html><html lang="en"><meta name="viewport" content="width=device-width"><title>Own Domains setup</title><body><main><h1>Phone setup</h1><ol><li>Use this Mac as the Wi-Fi DNS server.</li><li><a href="/ca.mobileconfig">Install on iPhone or iPad</a>, or <a href="/ca.crt">download the CA certificate</a> for other devices.</li><li>Install it and enable full SSL trust.</li></ol><p>The management dashboard remains available only on the Mac.</p></main></body></html>');
  });

  await new Promise<void>((resolve, reject) => {
    server.once('error', reject);
    server.listen(options.port, options.host, resolve);
  });
  const address = server.address();

  if (!address || 'string' === typeof address) throw new Error('Unable to determine setup server address');

  return {
    origin: `http://${options.host}:${address.port}`,
    close: () => new Promise((resolve, reject) => {
      server.close((error) => {
        if (error) reject(error);
        else resolve();
      });
    }),
  };
}
