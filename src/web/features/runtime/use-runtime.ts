import { useEffect, useSyncExternalStore } from 'react';
import type { RuntimeStore } from './runtime-store';

export function useRuntime(store: RuntimeStore) {
  const state = useSyncExternalStore(store.subscribe, store.getSnapshot);

  useEffect(() => {
    void store.connect();

    return () => { void store.disconnect() };
  }, [store]);

  return state;
}
