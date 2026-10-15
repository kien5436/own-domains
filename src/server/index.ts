import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import { join } from 'node:path';

import { DomainService } from './domains/service';
import { SettingsStore } from './domains/store';
import { createManagementServer } from './http/server';
import { createSetupServer } from './http/setup-server';
import { applyCaddyConfig } from './proxy/client';
import { createReconciler } from './proxy/reconcile';

const dataDirectory = join(homedir(), 'Library', 'Application Support', 'OwnDomains');

await mkdir(dataDirectory, { recursive: true, mode: 0o700 });
const appliedPath = join(dataDirectory, 'applied-revision');
const reconcile = createReconciler({
  apply: applyCaddyConfig,
  record: (revision) => writeFile(appliedPath, String(revision), { mode: 0o600 }),
  readApplied: async () => Number.parseInt(await readFile(appliedPath, 'utf8').catch(() => '0'), 10) || 0,
});
const store = new SettingsStore(dataDirectory);
const service = new DomainService(store, reconcile);
const settings = await store.load();

if (settings.lanAddress) await service.reconcileCurrent();
const readApplied = async () => Number.parseInt(await readFile(appliedPath, 'utf8').catch(() => '0'), 10) || 0;

await createSetupServer({ host: '127.0.0.1', port: 4781, caCertificatePath: join(homedir(), 'Library', 'Application Support', 'Caddy', 'pki', 'authorities', 'local', 'root.crt') });
const server = await createManagementServer({ service, store, host: '127.0.0.1', port: 4780, readApplied, staticDirectory: join(process.cwd(), 'dist') });

process.stdout.write(`Own Domains API listening at ${server.origin}\n`);
