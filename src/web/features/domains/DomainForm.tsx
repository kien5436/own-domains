import { useState, type FormEvent } from 'react';

import type { MappingInput } from '../../../shared/types';
import { isValidDomainInput, normalizeDomainInput } from './form-validation';

type DomainFormProps = {
  busy: boolean;
  onSubmit: (input: MappingInput) => Promise<void>;
};

export function DomainForm({ busy, onSubmit }: DomainFormProps) {
  const [hostname, setHostname] = useState('');
  const [port, setPort] = useState('3000');
  const [tls, setTls] = useState(false);
  const [error, setError] = useState('');

  async function submit(event: FormEvent) {
    event.preventDefault();

    if (!isValidDomainInput(hostname)) {
      setError('Enter a valid .own domain');

      return;
    }

    const upstreamPort = Number(port);

    if (!Number.isInteger(upstreamPort) || upstreamPort < 1 || upstreamPort > 65535) {
      setError('Enter a valid local port');

      return;
    }

    setError('');

    try {
      await onSubmit({ hostname: normalizeDomainInput(hostname), upstreamPort, tls, enabled: true });
      setHostname('');
    }
    catch (cause) {
      setError(cause instanceof Error ? cause.message : 'Unable to register domain');
    }
  }

  return <form className="grid gap-4 rounded-2xl border border-slate-200 bg-white p-5 shadow-sm" onSubmit={submit}>
    <div>
      <label className="mb-1 block text-sm font-medium text-slate-700" htmlFor="hostname">Domain</label>
      <div className="flex rounded-lg border border-slate-300 bg-white focus-within:border-blue-500 focus-within:ring-2 focus-within:ring-blue-200">
        <input aria-describedby="hostname-suffix" className="min-w-0 flex-1 rounded-l-lg px-3 py-2 focus:outline-none" id="hostname" onChange={(event) => setHostname(event.target.value)} placeholder="shop" value={hostname} />
        <span className="flex items-center rounded-r-lg border-l border-slate-300 bg-slate-50 px-3 text-slate-500" id="hostname-suffix">.own</span>
      </div>
    </div>
    <div>
      <label className="mb-1 block text-sm font-medium text-slate-700" htmlFor="port">Local port</label>
      <input className="w-full rounded-lg border border-slate-300 px-3 py-2" id="port" inputMode="numeric" onChange={(event) => setPort(event.target.value)} value={port} />
    </div>
    <label className="flex items-center gap-2 text-sm text-slate-700"><input checked={tls} onChange={(event) => setTls(event.target.checked)} type="checkbox" />Use HTTPS</label>
    {error && <p className="text-sm text-red-700" role="alert">{error}</p>}
    <button className="rounded-lg bg-blue-600 px-4 py-2 font-semibold text-white disabled:opacity-50" disabled={busy} type="submit">{busy ? 'Registering…' : 'Register domain'}</button>
  </form>;
}
