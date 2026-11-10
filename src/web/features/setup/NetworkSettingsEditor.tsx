import { useState, type FormEvent } from 'react';
import { errorMessage } from '../../lib/native-protocol';

export type NetworkState = { revision: number; lanAddress: string; upstreamDns: string[] };

export function NetworkSettingsEditor({ initial, busy, onSave }: { initial: NetworkState; busy: boolean; onSave: (state: NetworkState) => Promise<void> }) {
  const [state, setState] = useState(initial);
  const [message, setMessage] = useState('');

  async function submit(event: FormEvent) {
    event.preventDefault();

    try { await onSave(state); setMessage('Saved; check runtime status for activation.') }
    catch (error) { setMessage(errorMessage(error)) }
  }

  return <form className="grid gap-4" onSubmit={submit}>
    <label className="block text-sm font-medium text-slate-700 dark:text-slate-200" htmlFor="lan-address">Mac LAN IPv4</label>
    <input className="mt-1 w-full rounded-lg border border-slate-300 bg-transparent px-3 py-2 dark:border-slate-600" id="lan-address" onChange={(event) => setState({ ...state, lanAddress: event.target.value })} placeholder="192.168.1.20" value={state.lanAddress} />
    <label className="mt-3 block text-sm font-medium text-slate-700 dark:text-slate-200" htmlFor="upstream-dns">Upstream DNS</label>
    <input className="mt-1 w-full rounded-lg border border-slate-300 bg-transparent px-3 py-2 dark:border-slate-600" id="upstream-dns" onChange={(event) => setState({ ...state, upstreamDns: event.target.value.split(',').map((value) => value.trim()).filter(Boolean) })} value={state.upstreamDns.join(', ')} />
    {message && <p className="mt-3 text-sm text-slate-600 dark:text-slate-300" role="status">{message}</p>}
    <button disabled={busy} className="mt-4 rounded-lg bg-primary px-4 py-2 text-sm font-semibold text-white disabled:opacity-50" type="submit">Save network settings</button>
  </form>;
}
