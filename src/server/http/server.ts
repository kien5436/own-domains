import { randomBytes } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { createServer, type IncomingMessage, type ServerResponse } from 'node:http';
import { extname, join, normalize } from 'node:path';

import { RevisionConflictError, SettingsStore } from '../domains/store';
import { DomainService } from '../domains/service';
import { ValidationError } from '../domains/validation';
import { diagnoseNetwork } from '../network/diagnostics';

type ServerOptions = { service: DomainService; store: SettingsStore; host: string; port: number; readApplied?: () => Promise<number>; staticDirectory?: string };

export async function createManagementServer(options: ServerOptions): Promise<{ origin: string; close: () => Promise<void> }> {
  const csrfToken = randomBytes(24).toString('base64url');
  const sessionId = randomBytes(24).toString('base64url');
  const server = createServer(async (request, response) => {
    try {
      await route(request, response, options, { csrfToken, sessionId });
    }
    catch (error) {
      const status = error instanceof RevisionConflictError || /revision/i.test(String(error)) ? 409 : error instanceof ValidationError ? 400 : /not found/i.test(String(error)) ? 404 : 500;

      sendJson(response, status, { error: error instanceof Error ? error.message : 'Unexpected error' });
    }
  });

  await new Promise<void>((resolve, reject) => {
    server.once('error', reject);
    server.listen(options.port, options.host, resolve);
  });
  const address = server.address();

  if (!address || 'string' === typeof address) throw new Error('Unable to determine management address');
  const origin = `http://${options.host}:${address.port}`;

  return {
    origin,
    close: () => new Promise((resolve, reject) => {
      server.close((error) => {
        if (error) reject(error);
        else resolve();
      });
    }),
  };
}

async function route(request: IncomingMessage, response: ServerResponse, options: ServerOptions, session: { csrfToken: string; sessionId: string }) {
  const origin = `http://${request.headers.host}`;

  if (!isManagementHost(request.headers.host)) return sendJson(response, 403, { error: 'Invalid Host header' });
  if ('GET' === request.method && options.staticDirectory && !request.url?.startsWith('/api/')) return serveStatic(response, options.staticDirectory, request.url ?? '/');

  if ('/api/session' === request.url && 'GET' === request.method) {
    response.setHeader('Set-Cookie', `own_session=${session.sessionId}; HttpOnly; SameSite=Strict; Path=/`);

    return sendJson(response, 200, { csrfToken: session.csrfToken });
  }

  if (!request.headers.cookie?.includes(`own_session=${session.sessionId}`)) return sendJson(response, 403, { error: 'Local session required' });

  if ('GET' !== request.method)
    if (request.headers.origin !== origin || request.headers['x-csrf-token'] !== session.csrfToken) return sendJson(response, 403, { error: 'CSRF validation failed' });

  if ('/api/domains' === request.url && 'GET' === request.method) return sendJson(response, 200, await options.service.list());

  if ('/api/status' === request.url && 'GET' === request.method) {
    const settings = await options.service.list();
    const appliedRevision = await options.readApplied?.() ?? 0;

    return sendJson(response, 200, { lanAddress: settings.lanAddress, desiredRevision: settings.revision, appliedRevision, applyStatus: appliedRevision === settings.revision ? 'applied' : 'pending', diagnostics: await diagnoseNetwork(settings) });
  }

  if ('/api/settings/network' === request.url && 'GET' === request.method) {
    const settings = await options.service.list();

    return sendJson(response, 200, { revision: settings.revision, lanAddress: settings.lanAddress, upstreamDns: settings.upstreamDns });
  }

  if ('/api/settings/network' === request.url && 'PUT' === request.method) {
    const body = await readJson(request) as { expectedRevision: number; lanAddress: string; upstreamDns: string[] };
    const result = await options.service.updateNetwork({ lanAddress: body.lanAddress, upstreamDns: body.upstreamDns }, body.expectedRevision);

    return sendJson(response, 'applied' === result.apply.status ? 200 : 202, result);
  }

  if ('/api/domains' === request.url && 'POST' === request.method) {
    const body = await readJson(request) as { expectedRevision: number; mapping: Parameters<DomainService['create']>[0] };
    const result = await options.service.create(body.mapping, body.expectedRevision);

    return sendJson(response, 'applied' === result.apply.status ? 201 : 202, result);
  }

  if ('/api/reconcile' === request.url && 'POST' === request.method) {
    const result = await options.service.reconcileCurrent();

    return sendJson(response, 'applied' === result.status ? 200 : 202, result);
  }

  const match = request.url?.match(/^\/api\/domains\/([^/]+)$/);

  if (match && 'PATCH' === request.method) {
    const body = await readJson(request) as { expectedRevision: number; patch: Parameters<DomainService['update']>[1] };

    const result = await options.service.update(decodeURIComponent(match[1]!), body.patch, body.expectedRevision);

    return sendJson(response, 'applied' === result.apply.status ? 200 : 202, result);
  }

  if (match && 'DELETE' === request.method) {
    const body = await readJson(request) as { expectedRevision: number };

    const result = await options.service.remove(decodeURIComponent(match[1]!), body.expectedRevision);

    return sendJson(response, 'applied' === result.apply.status ? 200 : 202, result);
  }

  return sendJson(response, 404, { error: 'Not found' });
}

function isManagementHost(host: string | undefined): boolean {
  if ('registry.own' === host || 'registry.own:80' === host) return true;

  return /^127\.0\.0\.1:\d+$/.test(host ?? '') || /^localhost:\d+$/.test(host ?? '');
}

async function serveStatic(response: ServerResponse, directory: string, requestPath: string): Promise<void> {
  const path = '/' === requestPath ? 'index.html' : normalize(decodeURIComponent(requestPath.split('?')[0]!).replace(/^\/+/, ''));

  if (path.startsWith('..')) {
    sendJson(response, 404, { error: 'Not found' });

    return;
  }

  try {
    const body = await readFile(join(directory, path));
    const contentType = '.html' === extname(path) ? 'text/html; charset=utf-8' : '.css' === extname(path) ? 'text/css; charset=utf-8' : '.js' === extname(path) ? 'text/javascript; charset=utf-8' : 'application/octet-stream';

    response.writeHead(200, { 'content-type': contentType });
    response.end(body);
  }
  catch { sendJson(response, 404, { error: 'Not found' }) }
}

async function readJson(request: IncomingMessage): Promise<unknown> {
  const chunks: Buffer[] = [];
  let size = 0;

  for await (const chunk of request) {
    const buffer = Buffer.from(chunk);

    size += buffer.length;
    if (size > 64 * 1024) throw new ValidationError('Request body is too large');
    chunks.push(buffer);
  }

  try { return JSON.parse(Buffer.concat(chunks).toString('utf8')) }
  catch { throw new ValidationError('Request body must be valid JSON') }
}

function sendJson(response: ServerResponse, status: number, body: unknown) {
  response.writeHead(status, { 'content-type': 'application/json; charset=utf-8', 'cache-control': 'no-store' });
  response.end(JSON.stringify(body));
}
