import { applyUninstallPlan, buildUninstallPlan, type UninstallAction } from './uninstall-plan';

type UninstallOptions = { apply: boolean; manifestPath: string };
type UninstallResult = { exitCode: number; stdout: string; stderr: string };

export async function executeUninstall(options: UninstallOptions): Promise<UninstallResult> {
  const plan = await buildUninstallPlan(options.manifestPath);
  const preview = plan.actions.map(describeAction);

  if (!options.apply) return { exitCode: 0, stdout: `${preview.join('\n')}\n`, stderr: '' };
  await applyUninstallPlan(plan);
  const completed = plan.actions.map((action) => 'preserve' === action.kind ? describeAction(action) : `${'restore' === action.kind ? 'RESTORED' : 'REMOVED'} ${action.path}`);

  return { exitCode: 0, stdout: `${completed.join('\n')}\n`, stderr: '' };
}

export function parseUninstallArguments(args: string[]): { apply: boolean } {
  const normalizedArgs = args.filter((argument) => '--' !== argument);
  const known = new Set(['--apply', '--dry-run']);
  const unknown = normalizedArgs.find((argument) => argument.startsWith('--') && !known.has(argument));

  if (unknown) throw new Error(`Unknown argument: ${unknown}`);

  return { apply: normalizedArgs.includes('--apply') };
}

function describeAction(action: UninstallAction): string {
  if ('restore' === action.kind) return `RESTORE ${action.path} FROM ${action.backupPath}`;
  if ('remove' === action.kind) return `REMOVE ${action.path}`;

  return `PRESERVE ${action.path}: ${action.reason}`;
}
