import { createHash, randomUUID } from 'node:crypto';
import { chmod, lstat, open, readFile, rename, rm, stat } from 'node:fs/promises';
import { basename, dirname, join } from 'node:path';

type ManifestArtifact = { path: string; sha256: string };
type InstallManifest = { schemaVersion: 1; backupDirectory: string; artifacts: ManifestArtifact[] };

export type UninstallAction
  = | { kind: 'restore'; path: string; backupPath: string }
    | { kind: 'remove'; path: string }
    | { kind: 'preserve'; path: string; reason: string };

export type UninstallPlan = { manifestPath: string; actions: UninstallAction[] };

export async function buildUninstallPlan(manifestPath: string): Promise<UninstallPlan> {
  const manifest = parseManifest(await readFile(manifestPath, 'utf8'));
  const actions = await Promise.all(manifest.artifacts.map((artifact) => actionFor(artifact, manifest.backupDirectory)));

  return { manifestPath, actions: actions.filter((action): action is UninstallAction => Boolean(action)) };
}

export async function applyUninstallPlan(plan: UninstallPlan): Promise<void> {
  await applyActions(plan.actions, 0);

  if (!plan.actions.some((action) => 'preserve' === action.kind)) await rm(plan.manifestPath, { force: true });
}

async function actionFor(artifact: ManifestArtifact, backupDirectory: string): Promise<UninstallAction | undefined> {
  try {
    const metadata = await lstat(artifact.path);

    if (!metadata.isFile()) return { kind: 'preserve', path: artifact.path, reason: 'path is not a regular file' };
    const currentHash = createHash('sha256').update(await readFile(artifact.path)).digest('hex');

    if (currentHash !== artifact.sha256) return { kind: 'preserve', path: artifact.path, reason: 'content changed after setup' };
    const backupPath = join(backupDirectory, artifact.path.replace(/^\/+/, ''));

    return await isFile(backupPath) ? { kind: 'restore', path: artifact.path, backupPath } : { kind: 'remove', path: artifact.path };
  }
  catch (error) {
    if (isErrorCode(error, 'ENOENT')) return undefined;
    throw error;
  }
}

async function applyActions(actions: UninstallAction[], index: number): Promise<void> {
  const action = actions[index];

  if (!action) return;
  if ('restore' === action.kind) await atomicRestore(action.backupPath, action.path);
  else if ('remove' === action.kind) await rm(action.path, { force: true });
  await applyActions(actions, index + 1);
}

async function atomicRestore(source: string, target: string): Promise<void> {
  const [content, metadata] = await Promise.all([readFile(source), stat(source)]);
  const temporaryPath = join(dirname(target), `.${basename(target)}.${process.pid}.${randomUUID()}.tmp`);
  const file = await open(temporaryPath, 'wx', metadata.mode & 0o777);

  try {
    await file.writeFile(content);
    await file.sync();
  }
  finally {
    await file.close();
  }

  await rename(temporaryPath, target);
  await chmod(target, metadata.mode & 0o777);
}

async function isFile(path: string): Promise<boolean> {
  try {
    return (await lstat(path)).isFile();
  }
  catch (error) {
    if (isErrorCode(error, 'ENOENT')) return false;
    throw error;
  }
}

function parseManifest(content: string): InstallManifest {
  const value = JSON.parse(content) as Partial<InstallManifest>;

  if (1 !== value.schemaVersion || 'string' !== typeof value.backupDirectory || !Array.isArray(value.artifacts) || !value.artifacts.every((artifact) => artifact && 'string' === typeof artifact.path && /^[a-f\d]{64}$/.test(artifact.sha256))) throw new Error('Invalid Own Domains install manifest');

  return value as InstallManifest;
}

function isErrorCode(error: unknown, code: string): boolean {
  return error instanceof Error && 'code' in error && error.code === code;
}
