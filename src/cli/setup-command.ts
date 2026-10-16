import type { SetupPlan } from './setup-plan';
import { inspectSetupPlan, type SetupIssue } from './setup-preflight';
import { applySetupPlan } from './setup-writer';

type SetupCommandOptions = {
  apply: boolean;
  backupDirectory: string;
  plan: SetupPlan;
};

type SetupCommandDependencies = {
  activate: () => Promise<void>;
  deactivate: () => Promise<void>;
  inspect: (plan: SetupPlan) => Promise<SetupIssue[]>;
  write: typeof applySetupPlan;
};

type SetupCommandResult = { exitCode: number; stdout: string; stderr: string };

export type SetupArguments = { apply: boolean; lanAddress: string; upstreamDns: string[] };

const defaultDependencies: SetupCommandDependencies = { activate: () => Promise.resolve(), deactivate: () => Promise.resolve(), inspect: inspectSetupPlan, write: applySetupPlan };

export async function executeSetup(options: SetupCommandOptions, dependencies: Partial<SetupCommandDependencies> = {}): Promise<SetupCommandResult> {
  const { apply, backupDirectory, plan } = options;
  const services = { ...defaultDependencies, ...dependencies };
  const preview = plan.artifacts.map((artifact) => `CREATE ${artifact.path}`).join('\n');
  const issues = await services.inspect(plan);
  const errors = issues.filter((issue) => 'error' === issue.severity);
  const issueOutput = issues.map((issue) => `${issue.severity.toUpperCase()} ${issue.message}`).join('\n');

  if (!apply) return { exitCode: errors.length ? 2 : 0, stdout: `${preview}\n`, stderr: issueOutput ? `${issueOutput}\n` : '' };
  if (errors.length) return { exitCode: 2, stdout: `${preview}\n`, stderr: `${issueOutput}\n` };

  const result = await services.write(plan, backupDirectory);

  try {
    await services.activate();
  }
  catch (error) {
    await services.deactivate();
    await result.rollback();

    return { exitCode: 2, stdout: `${preview}\n`, stderr: `Service activation failed; setup files were rolled back: ${error instanceof Error ? error.message : String(error)}\n` };
  }

  const changes = [...result.written.map((path) => `WROTE ${path}`), ...result.skipped.map((path) => `UNCHANGED ${path}`), ...result.backups.map((path) => `BACKUP ${path}`)];

  return { exitCode: 0, stdout: `${preview}\n${changes.join('\n')}\n`, stderr: issueOutput ? `${issueOutput}\n` : '' };
}

export function parseSetupArguments(args: string[]): SetupArguments {
  const normalizedArgs = args.filter((argument) => '--' !== argument);
  const knownArguments = new Set(['--apply', '--dry-run', '--lan-address', '--upstream-dns']);
  const unknown = normalizedArgs.find((argument) => argument.startsWith('--') && !knownArguments.has(argument));

  if (unknown) throw new Error(`Unknown argument: ${unknown}`);
  const lanAddress = valueFor(normalizedArgs, '--lan-address');

  if (!lanAddress) throw new Error('--lan-address is required');
  const upstreamValue = valueFor(normalizedArgs, '--upstream-dns');
  const upstreamDns = upstreamValue ? upstreamValue.split(',').map((value) => value.trim()).filter(Boolean) : ['1.1.1.1', '9.9.9.9'];

  if (!upstreamDns.length) throw new Error('--upstream-dns must contain at least one address');

  return { apply: normalizedArgs.includes('--apply'), lanAddress, upstreamDns };
}

function valueFor(args: string[], name: string): string | undefined {
  const index = args.indexOf(name);

  if (-1 === index) return undefined;
  const value = args[index + 1];

  if (!value || value.startsWith('--')) throw new Error(`${name} requires a value`);

  return value;
}
