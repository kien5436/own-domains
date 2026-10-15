import { useState } from 'react';

import type { DomainMapping } from './api';

const RESERVED_PORTS = new Set([53, 80, 443, 4780]);

type Props = {
  mapping: DomainMapping;
  busy: boolean;
  onToggle: (mapping: DomainMapping) => Promise<void>;
  onSetTls: (mapping: DomainMapping) => Promise<void>;
  onUpdatePort: (mapping: DomainMapping, upstreamPort: number) => Promise<void>;
  onRemove: (mapping: DomainMapping) => Promise<void>;
};

export function DomainRow({ mapping, busy, onToggle, onSetTls, onUpdatePort, onRemove }: Props) {
  const [editing, setEditing] = useState(false);
  const [port, setPort] = useState(String(mapping.upstreamPort));
  const numericPort = Number(port);
  const validPort = Number.isInteger(numericPort) && 1 <= numericPort && 65535 >= numericPort && !RESERVED_PORTS.has(numericPort);

  async function save() {
    if (!validPort) return;
    await onUpdatePort(mapping, numericPort);
    setEditing(false);
  }

  return <li className="flex flex-wrap items-center gap-3 p-4">
    <div className="min-w-0 flex-1">
      <a className="font-semibold text-slate-900 hover:text-blue-700" href={`${mapping.tls ? 'https' : 'http'}://${mapping.hostname}`} rel="noreferrer" target="_blank">{mapping.hostname}</a>
      {editing
        ? <div className="mt-2 flex flex-wrap items-end gap-2">
          <label className="text-sm text-slate-700">Upstream port for {mapping.hostname}<input aria-invalid={!validPort} className="ml-2 w-24 rounded-lg border border-slate-300 px-2 py-1" inputMode="numeric" onChange={(event) => setPort(event.target.value)} value={port} /></label>
          <button className="rounded-lg bg-blue-600 px-3 py-1 text-sm text-white disabled:opacity-50" disabled={busy || !validPort} onClick={() => void save().catch(() => undefined)} type="button">Save {mapping.hostname}</button>
          <button className="rounded-lg border px-3 py-1 text-sm" disabled={busy} onClick={() => { setPort(String(mapping.upstreamPort)); setEditing(false) }} type="button">Cancel</button>
        </div>
        : <p className="text-sm text-slate-500">127.0.0.1:{mapping.upstreamPort} · {mapping.tls ? 'HTTPS' : 'HTTP'}</p>}
    </div>
    {!editing && <button aria-label={`Edit ${mapping.hostname}`} className="rounded-lg border px-3 py-1.5 text-sm" disabled={busy} onClick={() => setEditing(true)} type="button">Edit</button>}
    <button className="rounded-lg border px-3 py-1.5 text-sm" disabled={busy} onClick={() => void onToggle(mapping).catch(() => undefined)} type="button">{mapping.enabled ? 'Disable' : 'Enable'}</button>
    <button className="rounded-lg border px-3 py-1.5 text-sm" disabled={busy} onClick={() => void onSetTls(mapping).catch(() => undefined)} type="button">Turn TLS {mapping.tls ? 'off' : 'on'}</button>
    <button className="rounded-lg px-3 py-1.5 text-sm text-red-700" disabled={busy} onClick={() => { if (window.confirm(`Delete ${mapping.hostname}?`)) void onRemove(mapping).catch(() => undefined); }} type="button">Delete</button>
  </li>;
}
