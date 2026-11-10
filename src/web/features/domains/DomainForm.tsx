import { useState, type FormEvent } from 'react';

import type { MappingInput } from '../../../shared/types';
import { isValidDomainInput, normalizeDomainInput } from './form-validation';

type DomainFormProps = {
  busy: boolean;
  isTaken: (hostname: string) => boolean;
  onSubmit: (input: MappingInput) => Promise<void>;
};

export function DomainForm({ busy, isTaken, onSubmit }: DomainFormProps) {
  const [query, setQuery] = useState('');
  const [port, setPort] = useState('3000');
  const [tls, setTls] = useState(false);
  const [error, setError] = useState('');
  const [submitting, setSubmitting] = useState(false);

  const trimmed = query.trim();
  const normalized = '' === trimmed ? '' : normalizeDomainInput(trimmed);
  const valid = '' !== trimmed && isValidDomainInput(trimmed);
  const taken = valid && isTaken(normalized);
  const available = valid && !taken;

  async function submit(event: FormEvent) {
    event.preventDefault();

    if (submitting || !available) return;

    const upstreamPort = Number(port);

    if (!Number.isInteger(upstreamPort) || upstreamPort < 1 || upstreamPort > 65535) {
      setError('Enter a valid local port');

      return;
    }

    setError('');
    setSubmitting(true);

    try {
      await onSubmit({ hostname: normalized, upstreamPort, tls, enabled: true });
      setQuery('');
    }
    catch {
      // Submit failures surface through the global notification (the store
      // records them as state errors); showing them here too would duplicate
      // the message. The form keeps its input so the user can retry.
    }
    finally {
      setSubmitting(false);
    }
  }

  return <form className="grid gap-4" onSubmit={submit}>
    <div>
      <label className="mb-1 block text-sm font-medium text-slate-700 dark:text-slate-200" htmlFor="domain-search">Search domains</label>
      <div className="flex rounded-lg border border-slate-300 bg-white focus-within:border-primary focus-within:ring-2 focus-within:ring-primary-light dark:border-slate-600 dark:bg-slate-800">
        <input aria-describedby="domain-search-help" autoComplete="off" className="min-w-0 flex-1 rounded-l-lg bg-transparent px-3 py-2 focus:outline-none" id="domain-search" onChange={(event) => setQuery(event.target.value)} placeholder="Type a name…" value={query} />
        <span className="flex items-center rounded-r-lg border-l border-slate-300 bg-slate-50 px-3 text-slate-500 dark:border-slate-600 dark:bg-slate-700 dark:text-slate-300" id="domain-search-suffix">.own</span>
      </div>
      <p className="mt-2 text-sm text-slate-600 dark:text-slate-300" id="domain-search-help" role={'' !== trimmed && !valid ? 'alert' : undefined}>
        {'' === trimmed && 'Type a name to check availability.'}
        {'' !== trimmed && !valid && 'Enter a valid .own domain.'}
        {taken && `${normalized} is already registered.`}
        {available && `${normalized} is available.`}
      </p>
    </div>
    {available && <>
      <div>
        <label className="mb-1 block text-sm font-medium text-slate-700 dark:text-slate-200" htmlFor="port">Local port</label>
        <input className="w-full rounded-lg border border-slate-300 bg-transparent px-3 py-2 dark:border-slate-600" id="port" inputMode="numeric" onChange={(event) => setPort(event.target.value)} value={port} />
      </div>
      <label className="flex cursor-pointer items-center justify-between gap-2 text-sm text-slate-700 dark:text-slate-200" htmlFor="tls-toggle">Use HTTPS<input checked={tls} className="peer sr-only" id="tls-toggle" onChange={(event) => setTls(event.target.checked)} type="checkbox" /><span aria-hidden="true" className="relative h-6 w-11 rounded-full bg-slate-300 transition-colors peer-checked:bg-primary peer-focus-visible:ring-2 peer-focus-visible:ring-primary-light after:absolute after:left-0.5 after:top-0.5 after:h-5 after:w-5 after:rounded-full after:bg-white after:transition-transform peer-checked:after:translate-x-5 dark:bg-slate-600" /></label>
      <button className="rounded-lg bg-primary px-4 py-2 font-semibold text-white disabled:opacity-50" disabled={busy || submitting} type="submit">{submitting ? 'Registering…' : 'Register domain'}</button>
    </>}
    {error && <p className="text-sm text-red-700 dark:text-red-400" role="alert">{error}</p>}
  </form>;
}
