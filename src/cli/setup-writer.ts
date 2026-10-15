import { constants } from 'node:fs';
import { chmod, chown, copyFile, mkdir, open, readFile, rename, rm, stat } from 'node:fs/promises';
import { basename, dirname, join } from 'node:path';
import { randomUUID } from 'node:crypto';

import type { SetupPlan } from './setup-plan';

export type ApplySetupResult = { written: string[]; skipped: string[]; backups: string[] };

type PreviousState = { path: string; content?: Buffer; mode?: number; ownerId?: number };

export async function applySetupPlan(plan: SetupPlan, backupDirectory: string): Promise<ApplySetupResult> {
  const result: ApplySetupResult = { written: [], skipped: [], backups: [] };
  const previousStates: PreviousState[] = [];

  try {
    await applyArtifacts(plan.artifacts, 0, backupDirectory, result, previousStates);
  }
  catch (error) {
    await rollback(previousStates);
    throw error;
  }

  return result;
}

async function applyArtifacts(artifacts: SetupPlan['artifacts'], index: number, backupDirectory: string, result: ApplySetupResult, previousStates: PreviousState[]): Promise<void> {
  const artifact = artifacts[index];

  if (!artifact) return;
  const previous = await readPreviousState(artifact.path);

  if (previous.content?.equals(Buffer.from(artifact.content)) && previous.mode === artifact.mode) {
    result.skipped.push(artifact.path);
    await applyArtifacts(artifacts, index + 1, backupDirectory, result, previousStates);

    return;
  }

  if (previous.content && artifact.backupExisting) {
    const backupPath = join(backupDirectory, artifact.path.replace(/^\/+/, ''));

    await mkdir(dirname(backupPath), { recursive: true });

    try {
      await copyFile(artifact.path, backupPath, constants.COPYFILE_EXCL);
      result.backups.push(backupPath);
    }
    catch (error) {
      if (!isErrorCode(error, 'EEXIST')) throw error;
    }
  }

  previousStates.push(previous);
  await atomicWrite(artifact.path, artifact.content, artifact.mode, artifact.ownerId);
  result.written.push(artifact.path);
  await applyArtifacts(artifacts, index + 1, backupDirectory, result, previousStates);
}

async function readPreviousState(path: string): Promise<PreviousState> {
  try {
    const [content, metadata] = await Promise.all([readFile(path), stat(path)]);

    return { path, content, mode: metadata.mode & 0o777, ownerId: metadata.uid };
  }
  catch (error) {
    if (isErrorCode(error, 'ENOENT')) return { path };
    throw error;
  }
}

async function atomicWrite(path: string, content: string | Buffer, mode: number, ownerId?: number): Promise<void> {
  const parent = dirname(path);

  await mkdir(parent, { recursive: true });
  const temporaryPath = join(parent, `.${basename(path)}.${process.pid}.${randomUUID()}.tmp`);

  try {
    const file = await open(temporaryPath, 'wx', mode);

    try {
      await file.writeFile(content);
      await file.sync();
    }
    finally {
      await file.close();
    }

    await rename(temporaryPath, path);
    await chmod(path, mode);
    if (undefined !== ownerId) await chown(path, ownerId, -1);
  }
  catch (error) {
    await rm(temporaryPath, { force: true });
    throw error;
  }
}

async function rollback(previousStates: PreviousState[]): Promise<void> {
  const previous = previousStates.pop();

  if (!previous) return;
  if (previous.content && undefined !== previous.mode) await atomicWrite(previous.path, previous.content, previous.mode, previous.ownerId);
  else await rm(previous.path, { force: true });
  await rollback(previousStates);
}

function isErrorCode(error: unknown, code: string): boolean {
  return error instanceof Error && 'code' in error && error.code === code;
}
