// @vitest-environment jsdom
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { App } from '../../src/web/app/App';
import type { DomainApi } from '../../src/web/features/domains/api';

describe('App', () => {
  it('loads mappings and registers a domain through the feature API', async () => {
    const user = userEvent.setup();
    const api: DomainApi = {
      load: vi.fn().mockResolvedValue({ revision: 0, mappings: [] }),
      create: vi.fn().mockResolvedValue({ settings: { revision: 1, mappings: [{ id: '1', hostname: 'shop.own', upstreamPort: 3000, tls: false, enabled: true }] }, apply: { desiredRevision: 1, appliedRevision: 1, status: 'applied' } }),
      update: vi.fn(),
      remove: vi.fn(),
    };

    render(<App api={api} />);

    expect(await screen.findByText('No domains registered yet.')).toBeInTheDocument();
    await user.type(screen.getByLabelText('Domain'), 'shop.own');
    await user.click(screen.getByRole('button', { name: 'Register domain' }));

    expect(await screen.findByText('shop.own')).toBeInTheDocument();
    expect(api.create).toHaveBeenCalledWith(expect.objectContaining({ hostname: 'shop.own' }), 0);
  });

  it('edits the upstream port for an existing domain', async () => {
    const user = userEvent.setup();
    const mapping = { id: '1', hostname: 'shop.own', upstreamPort: 3000, tls: false, enabled: true };
    const api: DomainApi = {
      load: vi.fn().mockResolvedValue({ revision: 4, mappings: [mapping] }),
      create: vi.fn(),
      update: vi.fn().mockResolvedValue({
        settings: { revision: 5, mappings: [{ ...mapping, upstreamPort: 4173 }] },
        apply: { desiredRevision: 5, appliedRevision: 5, status: 'applied' },
      }),
      remove: vi.fn(),
    };

    render(<App api={api} />);

    await user.click(await screen.findByRole('button', { name: 'Edit shop.own' }));
    const port = screen.getByLabelText('Upstream port for shop.own');

    await user.clear(port);
    await user.type(port, '4173');
    await user.click(screen.getByRole('button', { name: 'Save shop.own' }));

    expect(api.update).toHaveBeenCalledWith('1', { upstreamPort: 4173 }, 4);
    expect(await screen.findByText('127.0.0.1:4173 · HTTP')).toBeInTheDocument();
  });
});
