import { useEffect } from 'react';

export type Notice = { id: string; message: string };

const AUTO_HIDE_MS = 5000;

export function Notifications({ notices, onDismiss }: { notices: Notice[]; onDismiss: (id: string) => void }) {
  useEffect(() => {
    if (0 === notices.length) return undefined;

    const timers = notices.map((notice) => window.setTimeout(() => onDismiss(notice.id), AUTO_HIDE_MS));

    return () => timers.forEach((timer) => window.clearTimeout(timer));
  }, [notices, onDismiss]);

  if (0 === notices.length) return null;

  return <div aria-live="polite" className="fixed right-4 top-4 z-50 grid w-80 gap-2">
    {notices.map((notice) => <div className="flex items-start gap-2 rounded-xl border border-slate-200 bg-white p-3 shadow-lg dark:border-slate-700 dark:bg-slate-900" key={notice.id} role="alert">
      <p className="min-w-0 flex-1 text-sm text-slate-800 dark:text-slate-100">{notice.message}</p>
      <button aria-label="Dismiss notification" className="rounded-md px-2 py-1 text-sm text-slate-500 hover:text-slate-800 dark:text-slate-400 dark:hover:text-slate-100" onClick={() => onDismiss(notice.id)} type="button">×</button>
    </div>)}
  </div>;
}
