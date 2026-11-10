import { useState } from 'react';
import { Pencil, Trash2 } from 'lucide-react';
import { openUrl } from '@tauri-apps/plugin-opener';

import type { DomainMapping } from '../../../shared/types';
import { Modal } from '../modal/Modal';

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
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const [port, setPort] = useState(String(mapping.upstreamPort));
  const numericPort = Number(port);
  const validPort = Number.isInteger(numericPort) && 1 <= numericPort && 65535 >= numericPort && !RESERVED_PORTS.has(numericPort);
  const url = `${mapping.tls ? 'https' : 'http'}://${mapping.hostname}`;

  async function save() {
    if (!validPort) return;

    await onUpdatePort(mapping, numericPort);
    setEditing(false);
  }

  async function openExternal(event: React.MouseEvent) {
    event.preventDefault();

    try {
      await openUrl(url);
    }
    catch {
      window.open(url, '_blank', 'noopener');
    }
  }

  function openEditor() {
    setPort(String(mapping.upstreamPort));
    setEditing(true);
  }

  return <li className="flex max-w-md flex-col gap-4 rounded-2xl border border-slate-200 bg-white p-5 shadow-sm dark:border-slate-700 dark:bg-slate-900">
    <div className="flex items-center justify-between gap-2">
      <h3 className="min-w-0 flex-1 truncate font-semibold text-slate-900 dark:text-white"><a className="hover:text-primary" href={url} onClick={(event) => void openExternal(event)} rel="noreferrer">{mapping.hostname}</a></h3>
    </div>
    <p className="text-sm text-slate-500 dark:text-slate-400">127.0.0.1:{mapping.upstreamPort}</p>
    <div aria-label={`Toggles for ${mapping.hostname}`} className="flex flex-wrap items-center gap-x-6 gap-y-2" role="group">
      <div className="flex items-center gap-2 text-sm text-slate-700 dark:text-slate-200"><span className="w-16 shrink-0" id={`enable-label-${mapping.id}`}>Enabled</span><button aria-checked={mapping.enabled} aria-label={`Enable ${mapping.hostname}`} className={`relative h-6 w-11 rounded-full transition-colors disabled:opacity-50 ${mapping.enabled ? 'bg-primary' : 'bg-slate-300 dark:bg-slate-600'}`} disabled={busy} onClick={() => void onToggle(mapping).catch(() => undefined)} role="switch" type="button"><span aria-hidden="true" className={`absolute left-0.5 top-0.5 h-5 w-5 rounded-full bg-white transition-transform ${mapping.enabled ? 'translate-x-5' : ''}`} /></button></div>
      <div className="flex items-center gap-2 text-sm text-slate-700 dark:text-slate-200"><span className="w-16 shrink-0" id={`tls-label-${mapping.id}`}>HTTPS</span><button aria-checked={mapping.tls} aria-label={`HTTPS for ${mapping.hostname}`} className={`relative h-6 w-11 rounded-full transition-colors disabled:opacity-50 ${mapping.tls ? 'bg-primary' : 'bg-slate-300 dark:bg-slate-600'}`} disabled={busy} onClick={() => void onSetTls(mapping).catch(() => undefined)} role="switch" type="button"><span aria-hidden="true" className={`absolute left-0.5 top-0.5 h-5 w-5 rounded-full bg-white transition-transform ${mapping.tls ? 'translate-x-5' : ''}`} /></button></div>
    </div>
    <div aria-label={`Actions for ${mapping.hostname}`} className="flex items-center justify-end gap-2 border-t border-slate-100 pt-4 dark:border-slate-800" role="group">
      <button aria-label={`Edit ${mapping.hostname}`} className="flex items-center gap-1 rounded-lg border border-slate-300 px-3 py-1.5 text-sm disabled:opacity-50 dark:border-slate-600" disabled={busy} onClick={openEditor} title={`Edit ${mapping.hostname}`} type="button"><Pencil aria-hidden="true" size={14} />Edit</button>
      <button aria-label={`Delete ${mapping.hostname}`} className="flex items-center gap-1 rounded-lg border border-red-300 px-3 py-1.5 text-sm text-red-700 disabled:opacity-50 dark:border-red-800 dark:text-red-400" disabled={busy} onClick={() => setConfirmingDelete(true)} title={`Delete ${mapping.hostname}`} type="button"><Trash2 aria-hidden="true" size={14} />Delete</button>
    </div>
    <Modal label={`Edit ${mapping.hostname}`} onClose={() => setEditing(false)} open={editing}>
      <label className="block text-sm text-slate-700 dark:text-slate-200" htmlFor={`port-${mapping.id}`}>Upstream port for {mapping.hostname}<input aria-invalid={!validPort} className="ml-2 w-24 rounded-lg border border-slate-300 bg-transparent px-2 py-1 dark:border-slate-600" disabled={busy} id={`port-${mapping.id}`} inputMode="numeric" onChange={(event) => setPort(event.target.value)} value={port} /></label>
      <div className="mt-4 flex justify-end gap-2">
        <button className="rounded-lg border border-slate-300 px-3 py-1.5 text-sm dark:border-slate-600" onClick={() => setEditing(false)} type="button">Cancel</button>
        <button className="rounded-lg bg-primary px-3 py-1.5 text-sm text-white hover:bg-primary-dark disabled:opacity-50" disabled={busy || !validPort} onClick={() => void save()} type="button">Save</button>
      </div>
    </Modal>
    <Modal label={`Delete ${mapping.hostname}?`} onClose={() => setConfirmingDelete(false)} open={confirmingDelete}>
      <p className="text-sm text-slate-600 dark:text-slate-300">This removes {mapping.hostname} from your routes. The app behind port {mapping.upstreamPort} keeps running.</p>
      <div className="mt-4 flex justify-end gap-2">
        <button className="rounded-lg border border-slate-300 px-3 py-1.5 text-sm dark:border-slate-600" onClick={() => setConfirmingDelete(false)} type="button">Cancel</button>
        <button className="rounded-lg bg-red-700 px-3 py-1.5 text-sm text-white hover:bg-red-800 disabled:opacity-50" disabled={busy} onClick={() => { setConfirmingDelete(false); void onRemove(mapping).catch(() => undefined) }} type="button">Confirm delete</button>
      </div>
    </Modal>
  </li>;
}
