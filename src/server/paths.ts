import { join } from 'node:path';

export function caddyRootCertificatePath(applicationDirectory: string): string {
  return join(applicationDirectory, 'caddy-data', 'caddy', 'pki', 'authorities', 'local', 'root.crt');
}
