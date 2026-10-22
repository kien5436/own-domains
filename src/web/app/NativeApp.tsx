import { useEffect, useState } from 'react';
import { DomainForm } from '../features/domains/DomainForm';
import { DomainTable } from '../features/domains/DomainTable';
import { DeviceInstructions } from '../features/setup/DeviceInstructions';
import { NetworkSettingsEditor } from '../features/setup/NetworkSettingsEditor';
import { StatusPanel } from '../features/status/StatusPanel';
import { useRuntime } from '../features/runtime/use-runtime';
import type { RuntimeStore } from '../features/runtime/runtime-store';
import { errorMessage } from '../lib/native-protocol';

export function NativeApp({ store, onStart, onStop, onQuit, onError }: { store: RuntimeStore; onStart: () => Promise<void>; onStop: () => Promise<void>; onQuit: () => Promise<void>; onError?: (handler: (error: unknown) => void) => Promise<() => void> }) {
  const state = useRuntime(store);
  const [actionError, setActionError] = useState('');

  useEffect(() => {
    let disposed = false;
    let cleanup: (() => void) | undefined;

    void onError?.((error) => { if (!disposed) setActionError(errorMessage(error)); }).then((unlisten) => { if (disposed) unlisten(); else cleanup = unlisten; }).catch((error) => { if (!disposed) setActionError(errorMessage(error)); });

    return () => { disposed = true; cleanup?.() };
  }, [onError]);
  const [acting, setActing] = useState(false);
  const snapshot = state.snapshot;
  const revision = snapshot?.settings.revision ?? 0;
  const busy = state.busy || acting || state.connection !== 'connected';
  const canStop = state.connection === 'connected' && snapshot !== null && snapshot.runtimeStatus !== 'stopped';

  async function action(operation: () => Promise<void>) {
    setActionError(''); setActing(true);

    try { await operation(); await store.connect() }
    catch (error) { setActionError(errorMessage(error)) }
    finally { setActing(false) }
  }

  async function startRouting() {
    setActionError(''); setActing(true);
    let startError: unknown;

    try {
      try { await onStart() }
      catch (error) { startError = error }

      // The daemon may be running even when start reported a runtime failure.
      // Reconnect so the user sees the current diagnostics instead of a stale disconnect.
      await store.connect();

      // A slow start can time out after the daemon has already applied the
      // configuration. Keep the warning only when the runtime is still unhealthy.
      if (startError && store.getSnapshot().snapshot?.runtimeStatus !== 'running') {
        setActionError(errorMessage(startError));
      }
    }
    catch (error) {
      setActionError(errorMessage(error));
    }
    finally {
      setActing(false);
    }
  }

  async function stopRouting() {
    setActionError(''); setActing(true);

    try {
      await onStop();
      // Stopping removes the daemon socket, so frontend cleanup may fail after routing stops.
      await store.disconnect().catch(() => undefined);
    }
    catch (error) {
      setActionError(errorMessage(error));
    }
    finally {
      setActing(false);
    }
  }

  return <main className="mx-auto min-h-screen max-w-5xl px-4 py-10 sm:px-6">
    <header className="mb-8"><p className="mb-2 text-sm font-semibold uppercase tracking-widest text-blue-700">Own Domains</p><h1 className="text-3xl font-bold tracking-tight text-slate-950">Local websites, memorable names.</h1></header>
    <div className="mb-6 flex flex-wrap items-center gap-4"><p role="status">Runtime: {state.connection === 'connected' ? snapshot?.runtimeStatus : state.connection}</p><button disabled={acting} onClick={() => void startRouting()} type="button">Start routing</button><button disabled={acting || !canStop} onClick={() => void stopRouting()} type="button">Stop routing</button><button disabled={acting} onClick={() => void action(onQuit)} type="button">Quit</button></div>
    {(actionError || state.error) && <p className="mb-4 rounded bg-red-50 p-3 text-red-800" role="alert">{actionError || state.error}</p>}
    <div className="grid gap-8 lg:grid-cols-[22rem_1fr]">
      <section><h2 className="mb-3 text-lg font-semibold">Register a domain</h2><DomainForm busy={busy} onSubmit={(input) => store.mutate({ type: 'create', payload: { input, expected_revision: revision } })} /></section>
      <section><h2 className="text-lg font-semibold">Your domains</h2><p>Revision {revision}</p>{snapshot
        ? <DomainTable busy={busy} mappings={snapshot.settings.mappings}
          onRemove={(mapping) => store.mutate({ type: 'remove', payload: { id: mapping.id, expected_revision: revision } })}
          onSetTls={(mapping) => store.mutate({ type: 'update', payload: { id: mapping.id, patch: { tls: !mapping.tls }, expected_revision: revision } })}
          onToggle={(mapping) => store.mutate({ type: 'update', payload: { id: mapping.id, patch: { enabled: !mapping.enabled }, expected_revision: revision } })}
          onUpdatePort={(mapping, upstreamPort) => store.mutate({ type: 'update', payload: { id: mapping.id, patch: { upstreamPort }, expected_revision: revision } })} />
        : <p>Connecting to daemon…</p>}</section>
    </div>
    {snapshot && <div className="mt-8 grid gap-8 md:grid-cols-2">
      <NetworkSettingsEditor busy={busy} initial={snapshot.settings} key={revision} onSave={(network) => store.mutate({ type: 'updateNetwork', payload: { lan_address: network.lanAddress, upstream_dns: network.upstreamDns, expected_revision: network.revision } })} />
      <StatusPanel appliedRevision={snapshot.appliedRevision} desiredRevision={snapshot.desiredRevision} diagnostics={snapshot.diagnostics} runtimeStatus={snapshot.runtimeStatus} />
      <DeviceInstructions lanAddress={snapshot.settings.lanAddress} />
    </div>}
  </main>;
}
