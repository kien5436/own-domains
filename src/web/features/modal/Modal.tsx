import { useEffect, useRef, type ReactNode } from 'react';
import { X } from 'lucide-react';

const FOCUSABLE = 'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])';

export function Modal({ open, onClose, label, children }: { open: boolean; onClose: () => void; label: string; children: ReactNode }) {
  const panel = useRef<HTMLDivElement>(null);
  const previouslyFocused = useRef<Element | null>(null);
  const onCloseRef = useRef(onClose);

  useEffect(() => {
    onCloseRef.current = onClose;
  });

  useEffect(() => {
    if (!open) return undefined;

    previouslyFocused.current = document.activeElement;

    const focusables = [...(panel.current?.querySelectorAll(FOCUSABLE) ?? [])].filter((element) => !(element as HTMLElement).hasAttribute('disabled')) as HTMLElement[];

    (focusables[0] ?? panel.current)?.focus();

    const onKeyDown = (event: KeyboardEvent) => {
      if ('Escape' === event.key) {
        onCloseRef.current();

        return;
      }

      if ('Tab' !== event.key) return;

      const elements = [...(panel.current?.querySelectorAll(FOCUSABLE) ?? [])].filter((element) => !(element as HTMLElement).hasAttribute('disabled')) as HTMLElement[];

      if (0 === elements.length) {
        event.preventDefault();

        return;
      }

      const first = elements[0];
      const last = elements[elements.length - 1];

      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      }
      else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    };

    document.addEventListener('keydown', onKeyDown);

    return () => {
      document.removeEventListener('keydown', onKeyDown);

      if (previouslyFocused.current instanceof HTMLElement) previouslyFocused.current.focus();
    };
  }, [open]);

  if (!open) return null;

  return <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-950/50 p-4" data-testid="modal-overlay" onClick={onClose}>
    <div aria-label={label} aria-modal="true" className="w-full max-w-md rounded-2xl border border-slate-200 bg-white p-5 shadow-xl dark:border-slate-700 dark:bg-slate-900" onClick={(event) => event.stopPropagation()} ref={panel} role="dialog" tabIndex={-1}>
      <div className="mb-4 flex items-center justify-between gap-2">
        <h2 className="text-lg font-semibold">{label}</h2>
        <button aria-label="Close" className="rounded-lg p-1.5 text-slate-500 hover:bg-slate-100 dark:text-slate-400 dark:hover:bg-slate-800" onClick={onClose} type="button"><X aria-hidden="true" size={18} /></button>
      </div>
      {children}
    </div>
  </div>;
}
