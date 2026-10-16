import { useCallback, useEffect, useState } from 'react';

import type { MappingInput } from '../../../shared/types';
import type { DomainApi, DomainMapping } from './api';

export function useDomains(api: DomainApi) {
  const [mappings, setMappings] = useState<DomainMapping[]>([]);
  const [revision, setRevision] = useState(0);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');

  const load = useCallback(async (preserveError = false) => {
    setLoading(true);

    try {
      const snapshot = await api.load();

      setMappings(snapshot.mappings);
      setRevision(snapshot.revision);
      if (!preserveError) setError('');
    }
    catch (cause) { setError(cause instanceof Error ? cause.message : 'Unable to load domains') }
    finally { setLoading(false) }
  }, [api]);

  useEffect(() => { void Promise.resolve().then(() => load()) }, [load]);

  async function mutate(operation: () => ReturnType<DomainApi['create']>, reportError = true) {
    setBusy(true);

    try {
      const result = await operation();

      setMappings(result.settings.mappings);
      setRevision(result.settings.revision);
      setError('error' === result.apply.status ? result.apply.message ?? 'Unable to apply configuration' : '');
    }
    catch (cause) {
      if (reportError) setError(cause instanceof Error ? cause.message : 'Request failed');
      await load(true);
      throw cause;
    }
    finally { setBusy(false) }
  }

  return {
    mappings,
    revision,
    loading,
    busy,
    error,
    create: (input: MappingInput) => mutate(() => api.create(input, revision), false),
    toggle: (mapping: DomainMapping) => mutate(() => api.update(mapping.id, { enabled: !mapping.enabled, tls: mapping.tls }, revision)),
    setTls: (mapping: DomainMapping) => mutate(() => api.update(mapping.id, { tls: !mapping.tls }, revision)),
    updatePort: (mapping: DomainMapping, upstreamPort: number) => mutate(() => api.update(mapping.id, { upstreamPort }, revision)),
    remove: (mapping: DomainMapping) => mutate(() => api.remove(mapping.id, revision)),
  };
}
