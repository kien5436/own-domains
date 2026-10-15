import type { Diagnostic } from '../../../shared/types';

export function StatusPanel({ diagnostics, desiredRevision, appliedRevision }: { diagnostics: Diagnostic[]; desiredRevision?: number; appliedRevision?: number }) {
  return <section aria-labelledby="status-heading" className="rounded-2xl border border-slate-200 bg-white p-5 shadow-sm">
    <h2 className="text-lg font-semibold" id="status-heading">System status</h2>
    {desiredRevision !== undefined && <p className="mt-1 text-sm text-slate-500">Configuration {appliedRevision === desiredRevision ? `applied at revision ${desiredRevision}` : `pending: desired ${desiredRevision}, applied ${appliedRevision ?? 0}`}</p>}
    {diagnostics.length === 0
      ? <p className="mt-2 text-sm text-slate-500">No diagnostics available yet.</p>
      : <ul className="mt-3 grid gap-3">{diagnostics.map((item) => <li className="rounded-xl bg-slate-50 p-3" key={item.code}>
        <p className="font-medium text-slate-900">{item.message}</p>
        {item.remedy && <p className="mt-1 text-sm text-slate-600">{item.remedy}</p>}
      </li>)}</ul>}
  </section>;
}
