import { DomainForm } from '../features/domains/DomainForm';
import { DomainTable } from '../features/domains/DomainTable';
import { domainApi, type DomainApi } from '../features/domains/api';
import { useDomains } from '../features/domains/use-domains';
import { DeviceInstructions } from '../features/setup/DeviceInstructions';
import { NetworkSettings } from '../features/setup/NetworkSettings';
import { StatusPanel } from '../features/status/StatusPanel';
import { useStatus } from '../features/status/use-status';

export function App({ api = domainApi }: { api?: DomainApi }) {
  const domains = useDomains(api);
  const status = useStatus();

  return <main className="mx-auto min-h-screen max-w-5xl px-4 py-10 sm:px-6">
    <header className="mb-8"><p className="mb-2 text-sm font-semibold uppercase tracking-widest text-blue-700">Own Domains</p><h1 className="text-3xl font-bold tracking-tight text-slate-950">Local websites, memorable names.</h1><p className="mt-2 max-w-2xl text-slate-600">Route any .own domain or subdomain to a local port, with optional trusted HTTPS.</p></header>
    <div className="grid gap-8 lg:grid-cols-[22rem_1fr]">
      <section aria-labelledby="register-heading"><h2 className="mb-3 text-lg font-semibold" id="register-heading">Register a domain</h2><DomainForm busy={domains.busy} onSubmit={domains.create} /></section>
      <section aria-labelledby="domains-heading"><div className="mb-3 flex items-center justify-between"><h2 className="text-lg font-semibold" id="domains-heading">Your domains</h2><span className="text-sm text-slate-500">Revision {domains.revision}</span></div>{domains.error && <p className="mb-3 rounded-lg bg-red-50 p-3 text-sm text-red-800" role="alert">{domains.error}</p>}{domains.loading ? <p>Loading domains…</p> : <DomainTable busy={domains.busy} mappings={domains.mappings} onRemove={domains.remove} onSetTls={domains.setTls} onToggle={domains.toggle} onUpdatePort={domains.updatePort} />}</section>
    </div>
    <div className="mt-8 grid gap-8 md:grid-cols-2"><NetworkSettings /><StatusPanel appliedRevision={status.appliedRevision} desiredRevision={status.desiredRevision} diagnostics={status.diagnostics} /><DeviceInstructions lanAddress={status.lanAddress} /></div>
  </main>;
}
