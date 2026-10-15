import { isAbsolute } from 'node:path';

import type { SetupPlan } from './setup-plan';

export type SetupIssue = { severity: 'error' | 'warning'; message: string };

export function inspectSetupPlan(plan: SetupPlan): Promise<SetupIssue[]> {
  const issues: SetupIssue[] = [];
  const paths = new Set<string>();

  for (const artifact of plan.artifacts) {
    if (!isAbsolute(artifact.path)) issues.push({ severity: 'error', message: `Setup path must be absolute: ${artifact.path}` });
    if (paths.has(artifact.path)) issues.push({ severity: 'error', message: `Duplicate setup path: ${artifact.path}` });
    if (!Number.isInteger(artifact.mode) || 0 > artifact.mode || 0o777 < artifact.mode) issues.push({ severity: 'error', message: `Invalid file mode for ${artifact.path}` });
    paths.add(artifact.path);
  }

  return Promise.resolve(issues);
}
