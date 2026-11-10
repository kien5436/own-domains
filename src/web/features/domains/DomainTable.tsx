import type { DomainMapping } from '../../../shared/types';
import { DomainRow } from './DomainRow';

type Props = { mappings: DomainMapping[]; busy: boolean; onToggle: (mapping: DomainMapping) => Promise<void>; onSetTls: (mapping: DomainMapping) => Promise<void>; onUpdatePort: (mapping: DomainMapping, upstreamPort: number) => Promise<void>; onRemove: (mapping: DomainMapping) => Promise<void> };

export function DomainTable({ mappings, busy, onToggle, onSetTls, onUpdatePort, onRemove }: Props) {
  if (!mappings.length) return <p className="rounded-2xl border border-dashed border-slate-300 p-8 text-center text-slate-500">No domains registered yet.</p>;

  return <ul className="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-4">{mappings.map((mapping) => <DomainRow busy={busy} key={mapping.id} mapping={mapping} onRemove={onRemove} onSetTls={onSetTls} onToggle={onToggle} onUpdatePort={onUpdatePort} />)}</ul>;
}
