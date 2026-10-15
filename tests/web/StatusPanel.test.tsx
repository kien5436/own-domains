// @vitest-environment jsdom
import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { StatusPanel } from '../../src/web/features/status/StatusPanel';

describe('StatusPanel', () => {
  it('shows remedies for failing diagnostics', () => {
    render(<StatusPanel diagnostics={[{ code: 'lan-address', status: 'warning', message: 'No LAN address selected', remedy: 'Select a LAN address.' }]} />);

    expect(screen.getByText('No LAN address selected')).toBeInTheDocument();
    expect(screen.getByText('Select a LAN address.')).toBeInTheDocument();
  });
});
