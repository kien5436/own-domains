import type { ApplyResult, Settings } from '../../shared/types';
import { buildProxyConfig } from './config';

type Dependencies = {
  apply: (config: object) => Promise<void>;
  record: (revision: number) => Promise<void>;
  readApplied?: () => Promise<number>;
};

export function createReconciler({ apply, record, readApplied = () => Promise.resolve(0) }: Dependencies) {
  return async (settings: Settings): Promise<ApplyResult> => {
    try {
      await apply(buildProxyConfig(settings));
      await record(settings.revision);

      return { desiredRevision: settings.revision, appliedRevision: settings.revision, status: 'applied' };
    }
    catch (error) {
      return { desiredRevision: settings.revision, appliedRevision: await readApplied(), status: 'error', message: error instanceof Error ? error.message : 'Unable to apply proxy configuration' };
    }
  };
}
