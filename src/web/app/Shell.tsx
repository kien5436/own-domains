import type { LucideIcon } from 'lucide-react';
import type { ReactNode } from 'react';

export type ShellSection = { id: string; label: string; Icon: LucideIcon };

export function Shell({ sections, active, onSelect, children }: { sections: ShellSection[]; active: string; onSelect: (id: string) => void; children: (active: string) => ReactNode }) {
  return <div className="mx-auto flex min-h-screen flex-col gap-6 px-4 py-10 sm:px-6 md:flex-row">
    <nav aria-label="Primary" className="md:sticky md:top-10 md:w-52 md:shrink-0 md:self-start">
      <p className="mb-3 text-sm font-semibold uppercase tracking-widest text-primary">Own Domains</p>
      <ul className="grid grid-cols-2 gap-2 md:flex md:flex-col">
        {sections.map(({ id, label, Icon }) => <li key={id}>
          <button
            aria-current={id === active ? 'page' : undefined}
            className={`flex w-full items-center gap-2 whitespace-nowrap rounded-lg px-3 py-2 text-sm font-medium ${id === active ? 'bg-primary-light text-slate-900 dark:bg-primary-dark dark:text-white' : 'text-slate-600 hover:bg-slate-100 dark:text-slate-300 dark:hover:bg-slate-800'}`}
            onClick={() => onSelect(id)}
            type="button"
          >
            <Icon aria-hidden="true" size={16} />
            {label}
          </button>
        </li>)}
      </ul>
    </nav>
    <div className="min-w-0 flex-1">{children(active)}</div>
  </div>;
}
