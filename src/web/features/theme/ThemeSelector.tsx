import { Moon, Sun, SunMoon } from 'lucide-react';

import { useTheme } from './use-theme';
import type { ThemePreference } from './theme';

const OPTIONS: { value: ThemePreference; label: string; Icon: typeof Sun }[] = [
  { value: 'system', label: 'System', Icon: SunMoon },
  { value: 'light', label: 'Light', Icon: Sun },
  { value: 'dark', label: 'Dark', Icon: Moon },
];

export function ThemeSelector() {
  const { preference, choose } = useTheme();

  return <fieldset>
    <legend className="mb-2 text-sm font-medium text-slate-700 dark:text-slate-200">Theme</legend>
    <div className="flex gap-2" role="radiogroup">
      {OPTIONS.map(({ value, label, Icon }) => <label className={`flex cursor-pointer items-center gap-2 rounded-lg border px-3 py-2 text-sm ${preference === value ? 'border-primary bg-primary-light dark:bg-primary-dark' : 'border-slate-300 dark:border-slate-600'}`} key={value}>
        <input checked={preference === value} className="sr-only" name="theme" onChange={() => choose(value)} type="radio" value={value} />
        <Icon aria-hidden="true" size={16} />
        {label}
      </label>)}
    </div>
  </fieldset>;
}
