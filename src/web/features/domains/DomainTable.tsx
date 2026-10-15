import type { DomainMapping } from './api';
import { DomainRow } from './DomainRow';

type Props = { mappings: DomainMapping[]; busy: boolean; onToggle: (mapping: DomainMapping) => Promise<void>; onSetTls: (mapping: DomainMapping) => Promise<void>; onUpdatePort: (mapping: DomainMapping, upstreamPort: number) => Promise<void>; onRemove: (mapping: DomainMapping) => Promise<void> };

export function DomainTable({ mappings, busy, onToggle, onSetTls, onUpdatePort, onRemove }: Props) {
  if (!mappings.length) return <p className="rounded-2xl border border-dashed border-slate-300 p-8 text-center text-slate-500">No domains registered yet.</p>;

  return <div className="overflow-hidden rounded-2xl border border-slate-200 bg-white shadow-sm">
    <ul className="divide-y divide-slate-200">{mappings.map((mapping) => <DomainRow busy={busy} key={mapping.id} mapping={mapping} onRemove={onRemove} onSetTls={onSetTls} onToggle={onToggle} onUpdatePort={onUpdatePort} />)}</ul>
  </div>;
}
