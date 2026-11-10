// @vitest-environment jsdom
import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { Notifications, type Notice } from '../../src/web/features/notifications/Notifications';

beforeEach(() => {
  vi.useFakeTimers();
});

describe('Notifications', () => {
  it('renders nothing when there are no notices', () => {
    const { container } = render(<Notifications notices={[]} onDismiss={() => undefined} />);

    expect(container.firstChild).toBeNull();
  });

  it('auto-hides a notice after five seconds', async () => {
    const onDismiss = vi.fn();
    const notices: Notice[] = [{ id: 'one', message: 'Routing started' }];

    render(<Notifications notices={notices} onDismiss={onDismiss} />);

    expect(screen.getByText('Routing started')).toBeInTheDocument();

    await vi.advanceTimersByTimeAsync(5000);

    expect(onDismiss).toHaveBeenCalledWith('one');
  });

  it('dismisses immediately from the close button', () => {
    const onDismiss = vi.fn();

    render(<Notifications notices={[{ id: 'one', message: 'Routing started' }]} onDismiss={onDismiss} />);

    fireEvent.click(screen.getByRole('button', { name: 'Dismiss notification' }));

    expect(onDismiss).toHaveBeenCalledWith('one');
  });
});
