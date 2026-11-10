// @vitest-environment jsdom
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { DomainForm } from '../../src/web/features/domains/DomainForm';

const AVAILABLE = () => false;

describe('DomainForm', () => {
  it('invites a search and reveals registration only for available names', async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn().mockResolvedValue(undefined);

    render(<DomainForm busy={false} isTaken={AVAILABLE} onSubmit={onSubmit} />);

    expect(screen.getByText('Type a name to check availability.')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Register domain' })).not.toBeInTheDocument();

    await user.type(screen.getByLabelText('Search domains'), 'Shop');

    expect(screen.getByText('shop.own is available.')).toBeInTheDocument();

    await user.clear(screen.getByLabelText('Local port'));
    await user.type(screen.getByLabelText('Local port'), '3000');
    await user.click(screen.getByLabelText('Use HTTPS'));
    await user.click(screen.getByRole('button', { name: 'Register domain' }));

    expect(onSubmit).toHaveBeenCalledWith({ hostname: 'shop.own', upstreamPort: 3000, tls: true, enabled: true });
  });

  it('reports taken names without a register action', async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn().mockResolvedValue(undefined);

    render(<DomainForm busy={false} isTaken={(hostname) => 'shop.own' === hostname} onSubmit={onSubmit} />);

    await user.type(screen.getByLabelText('Search domains'), 'shop');

    expect(screen.getByText('shop.own is already registered.')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Register domain' })).not.toBeInTheDocument();
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it('shows validation without submitting invalid input', async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();

    render(<DomainForm busy={false} isTaken={AVAILABLE} onSubmit={onSubmit} />);

    await user.type(screen.getByLabelText('Search domains'), '*');

    expect(screen.getByRole('alert')).toHaveTextContent('Enter a valid .own domain');
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it('rejects reserved names with help text', async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();

    render(<DomainForm busy={false} isTaken={AVAILABLE} onSubmit={onSubmit} />);

    await user.type(screen.getByLabelText('Search domains'), 'setup');

    expect(screen.getByRole('alert')).toHaveTextContent('Enter a valid .own domain');
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it('leaves submit failures to the notification instead of duplicating them inline', async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn().mockRejectedValue(new Error('pipe timeout'));

    render(<DomainForm busy={false} isTaken={AVAILABLE} onSubmit={onSubmit} />);

    await user.type(screen.getByLabelText('Search domains'), 'Shop');
    await user.click(screen.getByRole('button', { name: 'Register domain' }));

    await waitFor(() => expect(onSubmit).toHaveBeenCalled());
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    expect(screen.getByLabelText('Search domains')).toHaveValue('Shop');
  });
});
