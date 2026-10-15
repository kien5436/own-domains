import { describe, expect, it, vi } from 'vitest';

import { createReconciler } from '../../../src/server/proxy/reconcile';
import type { Settings } from '../../../src/shared/types';

const settings: Settings = { schemaVersion: 1, revision: 4, lanAddress: '192.168.1.20', upstreamDns: ['1.1.1.1'], mappings: [] };

describe('createReconciler', () => {
  it('records the applied revision after Caddy accepts the complete config', async () => {
    const apply = vi.fn().mockResolvedValue(undefined);
    const record = vi.fn().mockResolvedValue(undefined);
    const reconcile = createReconciler({ apply, record });

    await expect(reconcile(settings)).resolves.toEqual({ desiredRevision: 4, appliedRevision: 4, status: 'applied' });
    expect(record).toHaveBeenCalledWith(4);
  });

  it('reports an error and preserves the prior applied revision', async () => {
    const reconcile = createReconciler({ apply: vi.fn().mockRejectedValue(new Error('invalid config')), record: vi.fn(), readApplied: vi.fn().mockResolvedValue(3) });

    await expect(reconcile(settings)).resolves.toEqual({ desiredRevision: 4, appliedRevision: 3, status: 'error', message: 'invalid config' });
  });
});
