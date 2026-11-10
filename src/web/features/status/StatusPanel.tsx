import { useState } from 'react';
import { Stethoscope } from 'lucide-react';

import type { Diagnostic } from '../../../shared/types';

export function StatusPanel({ diagnostics, desiredRevision, appliedRevision, runtimeStatus, onRunDoctor, disabled }: { diagnostics: Diagnostic[]; desiredRevision?: number; appliedRevision?: number; runtimeStatus?: string; onRunDoctor: () => Promise<void>; disabled?: boolean }) {
  const [running, setRunning] = useState(false);

  async function run() {
    setRunning(true);

    try {
      await onRunDoctor();
    }
    finally {
      setRunning(false);
    }
  }

  return <section aria-labelledby="status-heading" className="rounded-2xl border border-slate-200 bg-white p-5 shadow-sm dark:border-slate-700 dark:bg-slate-900">
    <div className="flex items-center justify-between gap-2">
      <h2 className="text-lg font-semibold" id="status-heading">System status</h2>
      <button className="flex items-center gap-2 rounded-lg bg-primary px-3 py-1.5 text-sm font-semibold text-white disabled:opacity-50" disabled={disabled || running} onClick={() => void run()} type="button"><Stethoscope aria-hidden="true" size={14} />{running ? 'Running…' : 'Run doctor'}</button>
    </div>
    {desiredRevision !== undefined && <p className="mt-1 text-sm text-slate-500 dark:text-slate-400">Configuration {appliedRevision === desiredRevision && (runtimeStatus === undefined || runtimeStatus === 'running') ? `applied at revision ${desiredRevision}` : `pending: desired ${desiredRevision}, applied ${appliedRevision ?? 0}`}</p>}
    {diagnostics.length === 0
      ? <p className="mt-3 rounded-xl bg-primary-light p-3 text-sm text-slate-800 dark:bg-primary-dark dark:text-slate-100">All systems healthy — no issues found.</p>
      : <ul className="mt-3 grid gap-3">{diagnostics.map((item) => <li className="rounded-xl bg-slate-50 p-3 dark:bg-slate-800" key={item.code}>
        <p className="font-medium text-slate-900 dark:text-white">{item.message}</p>
        {item.remedy && <p className="mt-1 text-sm text-slate-600 dark:text-slate-300">{item.remedy}</p>}
      </li>)}</ul>}
  </section>;
}
