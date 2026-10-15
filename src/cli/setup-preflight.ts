import { execFile } from 'node:child_process';
import { lstat, readFile } from 'node:fs/promises';
import { isAbsolute } from 'node:path';

import type { SetupArtifact, SetupPlan } from './setup-plan';

export type SetupIssue = { severity: 'error' | 'warning'; message: string };

export type PortListener = { address: string; port: number; process: string };

type SetupPreflightOptions = {
  inspectListeners: () => Promise<PortListener[]>;
  listenAddresses: string[];
  userId: number;
};

const defaultOptions: SetupPreflightOptions = {
  inspectListeners,
  listenAddresses: ['127.0.0.1'],
  userId: process.getuid?.() ?? 0,
};

export async function inspectSetupPlan(plan: SetupPlan, options: Partial<SetupPreflightOptions> = {}): Promise<SetupIssue[]> {
  const settings = { ...defaultOptions, ...options };
  const issues = structuralIssues(plan);
  const ownedPaths = await readOwnedPaths(plan);
  const artifactIssues = await Promise.all(plan.artifacts.map((artifact) => inspectArtifact(artifact, settings.userId, ownedPaths)));
  const listeners = await settings.inspectListeners();

  issues.push(...artifactIssues.flat(), ...listenerIssues(listeners, settings.listenAddresses));

  return issues;
}

function structuralIssues(plan: SetupPlan): SetupIssue[] {
  const issues: SetupIssue[] = [];
  const paths = new Set<string>();

  for (const artifact of plan.artifacts) {
    if (!isAbsolute(artifact.path)) issues.push({ severity: 'error', message: `Setup path must be absolute: ${artifact.path}` });
    if (paths.has(artifact.path)) issues.push({ severity: 'error', message: `Duplicate setup path: ${artifact.path}` });
    if (!Number.isInteger(artifact.mode) || 0 > artifact.mode || 0o777 < artifact.mode) issues.push({ severity: 'error', message: `Invalid file mode for ${artifact.path}` });
    paths.add(artifact.path);
  }

  return issues;
}

async function inspectArtifact(artifact: SetupArtifact, userId: number, ownedPaths: Set<string>): Promise<SetupIssue[]> {
  try {
    const metadata = await lstat(artifact.path);

    if (!metadata.isFile()) return [{ severity: 'error', message: `Setup path is not a regular file: ${artifact.path}` }];
    const issues: SetupIssue[] = [];
    const expectedOwner = isSystemPath(artifact.path) ? 0 : userId;

    if (metadata.uid !== expectedOwner) issues.push({ severity: 'error', message: `Unexpected owner for ${artifact.path}; expected uid ${expectedOwner}` });
    const content = await readFile(artifact.path, 'utf8');

    if (content !== artifact.content) issues.push(ownedPaths.has(artifact.path)
      ? { severity: 'warning', message: `Managed setup file will be updated: ${artifact.path}` }
      : { severity: 'error', message: `Existing file conflicts with setup: ${artifact.path}` });

    return issues;
  }
  catch (error) {
    if (isErrorCode(error, 'ENOENT')) return [];
    throw error;
  }
}

function listenerIssues(listeners: PortListener[], listenAddresses: string[]): SetupIssue[] {
  return listeners.flatMap((listener) => {
    const expectedProcess = 53 === listener.port ? 'dnsmasq' : 'caddy';
    const requiredAddress = '*' === listener.address || listenAddresses.includes(listener.address);

    if (![53, 80, 443].includes(listener.port) || !requiredAddress || listener.process.toLowerCase().includes(expectedProcess)) return [];

    return [{ severity: 'error' as const, message: `${listener.process} occupies required port ${listener.port} on ${listener.address}` }];
  });
}

async function readOwnedPaths(plan: SetupPlan): Promise<Set<string>> {
  const manifest = plan.artifacts.find((artifact) => artifact.path.endsWith('/install-manifest.json'));

  if (!manifest) return new Set();

  try {
    const parsed = JSON.parse(await readFile(manifest.path, 'utf8')) as { ownedPaths?: unknown };

    return new Set(Array.isArray(parsed.ownedPaths) ? parsed.ownedPaths.filter((path): path is string => 'string' === typeof path) : []);
  }
  catch (error) {
    if (isErrorCode(error, 'ENOENT') || error instanceof SyntaxError) return new Set();
    throw error;
  }
}

function inspectListeners(): Promise<PortListener[]> {
  return new Promise((resolve) => {
    execFile('/usr/sbin/lsof', ['-nP', '-iTCP:53', '-iUDP:53', '-iTCP:80', '-iTCP:443'], { timeout: 1_500 }, (error, stdout) => {
      if (error && !stdout) {
        resolve([]);

        return;
      }

      resolve(parsePortListeners(stdout));
    });
  });
}

export function parsePortListeners(output: string): PortListener[] {
  return output.split('\n').flatMap((line) => {
    const [processName] = line.trim().split(/\s+/);
    const endpoint = line.match(/(?:TCP|UDP)\s+(\*|(?:\d{1,3}\.){3}\d{1,3}|\[::\]):(53|80|443)\b/);

    return processName && endpoint?.[1] && endpoint[2] ? [{ process: processName, address: endpoint[1], port: Number(endpoint[2]) }] : [];
  });
}

function isSystemPath(path: string): boolean {
  return path.startsWith('/etc/') || path.startsWith('/Library/');
}

function isErrorCode(error: unknown, code: string): boolean {
  return error instanceof Error && 'code' in error && error.code === code;
}
