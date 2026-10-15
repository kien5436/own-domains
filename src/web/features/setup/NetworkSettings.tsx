import { useEffect, useState, type FormEvent } from 'react';

import type { ApplyResult } from '../../../shared/types';
import { apiRequest } from '../../lib/api-client';

type NetworkState = { revision: number; lanAddress: string; upstreamDns: string[] };

export function NetworkSettings() {
  const [state, setState] = useState<NetworkState>({ revision: 0, lanAddress: '', upstreamDns: ['1.1.1.1', '9.9.9.9'] });
  const [message, setMessage] = useState('');

  useEffect(() => {
    void Promise.resolve().then(() => apiRequest<NetworkState>('/api/settings/network')).then(setState, (error) => setMessage(error instanceof Error ? error.message : 'Unable to load network settings'));
  }, []);

  async function submit(event: FormEvent) {
    event.preventDefault();

    try {
      const result = await apiRequest<{ settings: NetworkState; apply: ApplyResult }>('/api/settings/network', {
        method: 'PUT',
        body: JSON.stringify({ expectedRevision: state.revision, lanAddress: state.lanAddress, upstreamDns: state.upstreamDns }),
      });

      setState(result.settings);
      setMessage('applied' === result.apply.status ? 'Network configuration applied.' : result.apply.message ?? 'Saved; configuration is pending.');
    }
    catch (error) { setMessage(error instanceof Error ? error.message : 'Unable to save network settings') }
  }

  return <form className="rounded-2xl border border-slate-200 bg-white p-5 shadow-sm" onSubmit={submit}>
    <h2 className="text-lg font-semibold">Network settings</h2>
    <label className="mt-3 block text-sm font-medium" htmlFor="lan-address">Mac LAN IPv4</label>
    <input className="mt-1 w-full rounded-lg border border-slate-300 px-3 py-2" id="lan-address" onChange={(event) => setState({ ...state, lanAddress: event.target.value })} placeholder="192.168.1.20" value={state.lanAddress} />
    <label className="mt-3 block text-sm font-medium" htmlFor="upstream-dns">Upstream DNS</label>
    <input className="mt-1 w-full rounded-lg border border-slate-300 px-3 py-2" id="upstream-dns" onChange={(event) => setState({ ...state, upstreamDns: event.target.value.split(',').map((value) => value.trim()).filter(Boolean) })} value={state.upstreamDns.join(', ')} />
    {message && <p className="mt-3 text-sm text-slate-600" role="status">{message}</p>}
    <button className="mt-4 rounded-lg bg-slate-900 px-4 py-2 text-sm font-semibold text-white" type="submit">Save network settings</button>
  </form>;
}
