// @vitest-environment jsdom
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { DomainForm } from '../../src/web/features/domains/DomainForm';

describe('DomainForm', () => {
  it('submits a normalized domain mapping', async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn().mockResolvedValue(undefined);

    render(<DomainForm busy={false} onSubmit={onSubmit} />);

    await user.type(screen.getByLabelText('Domain'), 'Shop.OWN.');
    await user.clear(screen.getByLabelText('Local port'));
    await user.type(screen.getByLabelText('Local port'), '3000');
    await user.click(screen.getByLabelText('Use HTTPS'));
    await user.click(screen.getByRole('button', { name: 'Register domain' }));

    expect(onSubmit).toHaveBeenCalledWith({ hostname: 'shop.own', upstreamPort: 3000, tls: true, enabled: true });
  });

  it('shows validation without submitting invalid input', async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();

    render(<DomainForm busy={false} onSubmit={onSubmit} />);

    await user.type(screen.getByLabelText('Domain'), '*.own');
    await user.click(screen.getByRole('button', { name: 'Register domain' }));

    expect(await screen.findByRole('alert')).toHaveTextContent('Enter a valid .own domain');
    expect(onSubmit).not.toHaveBeenCalled();
  });
});
