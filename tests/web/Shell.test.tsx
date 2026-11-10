// @vitest-environment jsdom
import { fireEvent, render, screen } from '@testing-library/react';
import { BookOpen, LayoutDashboard, Settings as SettingsIcon, Stethoscope } from 'lucide-react';
import { describe, expect, it } from 'vitest';

import { Shell, type ShellSection } from '../../src/web/app/Shell';

const SECTIONS: ShellSection[] = [
  { id: 'dashboard', label: 'Dashboard', Icon: LayoutDashboard },
  { id: 'settings', label: 'Settings', Icon: SettingsIcon },
  { id: 'guide', label: 'User guide', Icon: BookOpen },
  { id: 'diagnostic', label: 'Diagnostic', Icon: Stethoscope },
];

const PANELS: Record<string, string> = {
  dashboard: 'Dashboard panel',
  settings: 'Settings panel',
  guide: 'Guide panel',
  diagnostic: 'Diagnostic panel',
};

function renderShell() {
  return render(<Shell sections={SECTIONS} active="dashboard" onSelect={() => undefined}>
    {(id) => <p>{PANELS[id]}</p>}
  </Shell>);
}

describe('Shell', () => {
  it('renders all four navigation items', () => {
    renderShell();

    for (const section of SECTIONS) expect(screen.getByRole('button', { name: section.label })).toBeInTheDocument();
  });

  it('shows the active section panel and marks its nav item', () => {
    const { rerender } = renderShell();

    expect(screen.getByText('Dashboard panel')).toBeInTheDocument();

    rerender(<Shell sections={SECTIONS} active="settings" onSelect={() => undefined}>
      {(id) => <p>{PANELS[id]}</p>}
    </Shell>);

    expect(screen.getByText('Settings panel')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Settings' })).toHaveAttribute('aria-current', 'page');
  });

  it('keeps the nav fixed while the content scrolls', () => {
    renderShell();

    expect(screen.getByRole('navigation')).toHaveClass('md:sticky');
  });

  it('notifies when a nav item is selected', () => {
    let selected = '';

    render(<Shell sections={SECTIONS} active="dashboard" onSelect={(id) => { selected = id }}>
      {(id) => <p>{PANELS[id]}</p>}
    </Shell>);

    fireEvent.click(screen.getByRole('button', { name: 'Diagnostic' }));

    expect(selected).toBe('diagnostic');
  });
});
