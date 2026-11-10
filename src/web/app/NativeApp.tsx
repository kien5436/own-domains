import { useEffect, useState } from 'react';
import { BookOpen, LayoutDashboard, Play, Power, Settings as SettingsIcon, Square, Stethoscope } from 'lucide-react';
import { DomainForm } from '../features/domains/DomainForm';
import { DomainTable } from '../features/domains/DomainTable';
import logoDarkUrl from '../../../crates/own-domains-desktop/icons/icon.png';
import logoLightUrl from '../assets/logo-light.png';
import { DeviceInstructions } from '../features/setup/DeviceInstructions';
import { NetworkSettingsEditor } from '../features/setup/NetworkSettingsEditor';
import { Notifications } from '../features/notifications/Notifications';
import { ThemeSelector } from '../features/theme/ThemeSelector';
import { StatusPanel } from '../features/status/StatusPanel';
import { Shell } from './Shell';
import { useRuntime } from '../features/runtime/use-runtime';
import { useTheme } from '../features/theme/use-theme';
import type { RuntimeStore } from '../features/runtime/runtime-store';
import { errorMessage } from '../lib/native-protocol';

export function NativeApp({ store, onStart, onStop, onQuit, onError }: { store: RuntimeStore; onStart: () => Promise<void>; onStop: () => Promise<void>; onQuit: () => Promise<void>; onError?: (handler: (error: unknown) => void) => Promise<() => void> }) {
  const state = useRuntime(store);
  const { resolved } = useTheme();
  const logoUrl = 'dark' === resolved ? logoDarkUrl : logoLightUrl;
  const [actionError, setActionError] = useState('');

  useEffect(() => {
    let disposed = false;
    let cleanup: (() => void) | undefined;

    void onError?.((error) => { if (!disposed) setActionError(errorMessage(error)); }).then((unlisten) => { if (disposed) unlisten(); else cleanup = unlisten; }).catch((error) => { if (!disposed) setActionError(errorMessage(error)); });

    return () => { disposed = true; cleanup?.() };
  }, [onError]);
  const [acting, setActing] = useState(false);
  const [section, setSection] = useState('dashboard');
  const [dismissed, setDismissed] = useState<string[]>([]);
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

  return <>
    <Notifications notices={notices()} onDismiss={(id) => setDismissed((current) => [...current, id])} />
    <Shell active={section} onSelect={setSection} sections={SECTIONS}>
      {(id) => 'dashboard' === id ? dashboard() : 'settings' === id ? settings() : 'guide' === id ? guide() : diagnostic()}
    </Shell>
  </>;

  function notices() {
    const all = [
      '' !== actionError && { id: `action:${actionError}`, message: actionError },
      null !== state.error && '' !== state.error && { id: `connection:${state.error}`, message: state.error },
    ];

    return all.filter((notice): notice is { id: string; message: string } => false !== notice && !dismissed.includes(notice.id));
  }

  function dashboard() {
    return <>
      <header className="mb-8 flex items-center gap-3"><img alt="Own Domains" className="h-10 w-10 rounded-xl" src={logoUrl} /><h1 className="text-2xl font-bold tracking-tight text-slate-950 sm:text-3xl dark:text-white">Local websites, memorable names.</h1></header>
      <div className="mb-6 flex flex-wrap items-center gap-3">{canStop
        ? <button aria-label="Stop routing" className="flex items-center gap-2 rounded-lg bg-primary px-4 py-2 font-semibold text-white hover:bg-primary-dark disabled:opacity-50" disabled={acting} onClick={() => void stopRouting()} type="button"><Square aria-hidden="true" size={16} />Stop</button>
        : <button aria-label="Start routing" className="flex items-center gap-2 rounded-lg bg-primary px-4 py-2 font-semibold text-white hover:bg-primary-dark disabled:opacity-50" disabled={acting} onClick={() => void startRouting()} type="button"><Play aria-hidden="true" size={16} />Start</button>}
      <button aria-label="Quit" className="flex items-center gap-2 rounded-lg border border-slate-300 px-4 py-2 font-semibold text-slate-600 hover:bg-slate-100 dark:border-slate-700 dark:text-slate-300 dark:hover:bg-slate-800" disabled={acting} onClick={() => void action(onQuit)} title="Quit" type="button"><Power aria-hidden="true" size={16} />Quit</button></div>
      <div className="grid items-start gap-6">
        <section className="rounded-2xl border border-slate-200 bg-white p-5 shadow-sm dark:border-slate-700 dark:bg-slate-900"><h2 className="mb-3 text-lg font-semibold">Register a domain</h2><DomainForm busy={busy} isTaken={(hostname) => snapshot?.settings.mappings.some((mapping) => mapping.hostname === hostname) ?? false} onSubmit={(input) => store.mutate({ type: 'create', payload: { input, expected_revision: revision } })} /></section>
        <section className="rounded-2xl border border-slate-200 bg-white p-5 shadow-sm dark:border-slate-700 dark:bg-slate-900"><h2 className="mb-3 text-lg font-semibold">Your domains</h2>{snapshot
          ? <DomainTable busy={busy} mappings={snapshot.settings.mappings}
            onRemove={(mapping) => store.mutate({ type: 'remove', payload: { id: mapping.id, expected_revision: revision } })}
            onSetTls={(mapping) => store.mutate({ type: 'update', payload: { id: mapping.id, patch: { tls: !mapping.tls }, expected_revision: revision } })}
            onToggle={(mapping) => store.mutate({ type: 'update', payload: { id: mapping.id, patch: { enabled: !mapping.enabled }, expected_revision: revision } })}
            onUpdatePort={(mapping, upstreamPort) => store.mutate({ type: 'update', payload: { id: mapping.id, patch: { upstreamPort }, expected_revision: revision } })} />
          : <p>Connecting to daemon…</p>}</section>
      </div>
    </>;
  }

  function settings() {
    if (!snapshot) return <p>Connecting to daemon…</p>;

    return <div className="grid items-start gap-6">
      <section className="rounded-2xl border border-slate-200 bg-white p-5 shadow-sm dark:border-slate-700 dark:bg-slate-900"><h2 className="mb-3 text-lg font-semibold">Network settings</h2><NetworkSettingsEditor busy={busy} initial={snapshot.settings} key={revision} onSave={(network) => store.mutate({ type: 'updateNetwork', payload: { lan_address: network.lanAddress, upstream_dns: network.upstreamDns, expected_revision: network.revision } })} /></section>
      <section className="rounded-2xl border border-slate-200 bg-white p-5 shadow-sm dark:border-slate-700 dark:bg-slate-900"><h2 className="mb-3 text-lg font-semibold">Appearance</h2><ThemeSelector /></section>
    </div>;
  }

  function guide() {
    return <DeviceInstructions lanAddress={snapshot?.settings.lanAddress ?? ''} />;
  }

  function diagnostic() {
    if (!snapshot) return <p>Connecting to daemon…</p>;

    return <StatusPanel appliedRevision={snapshot.appliedRevision} desiredRevision={snapshot.desiredRevision} diagnostics={snapshot.diagnostics} disabled={state.connection !== 'connected'} onRunDoctor={() => store.mutate({ type: 'doctor' })} runtimeStatus={snapshot.runtimeStatus} />;
  }
}

const SECTIONS = [
  { id: 'dashboard', label: 'Dashboard', Icon: LayoutDashboard },
  { id: 'settings', label: 'Settings', Icon: SettingsIcon },
  { id: 'guide', label: 'User guide', Icon: BookOpen },
  { id: 'diagnostic', label: 'Diagnostic', Icon: Stethoscope },
];
