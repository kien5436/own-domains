// @vitest-environment jsdom
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { StatusPanel } from '../../src/web/features/status/StatusPanel';

describe('StatusPanel', () => {
  it('shows remedies for failing diagnostics', () => {
    render(<StatusPanel diagnostics={[{ code: 'lan-address', status: 'warning', message: 'No LAN address selected', remedy: 'Select a LAN address.' }]} onRunDoctor={async () => undefined} />);

    expect(screen.getByText('No LAN address selected')).toBeInTheDocument();
    expect(screen.getByText('Select a LAN address.')).toBeInTheDocument();
  });

  it('reports healthy state when there are no diagnostics', () => {
    render(<StatusPanel diagnostics={[]} onRunDoctor={async () => undefined} />);

    expect(screen.getByText(/all systems healthy/i)).toBeInTheDocument();
  });

  it('runs doctor from the button', () => {
    const onRunDoctor = vi.fn().mockResolvedValue(undefined);

    render(<StatusPanel diagnostics={[]} onRunDoctor={onRunDoctor} />);

    fireEvent.click(screen.getByRole('button', { name: 'Run doctor' }));

    expect(onRunDoctor).toHaveBeenCalledOnce();
  });

  it('disables the doctor button when disconnected', () => {
    const onRunDoctor = vi.fn().mockResolvedValue(undefined);

    render(<StatusPanel diagnostics={[]} disabled onRunDoctor={onRunDoctor} />);

    expect(screen.getByRole('button', { name: 'Run doctor' })).toBeDisabled();
  });
});
