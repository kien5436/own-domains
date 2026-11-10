// @vitest-environment jsdom
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { DomainMapping } from '../../src/shared/types';
import { DomainTable } from '../../src/web/features/domains/DomainTable';

const MAPPINGS: DomainMapping[] = [
  { id: 'one', hostname: 'shop.own', upstreamPort: 3000, tls: true, enabled: true },
  { id: 'two', hostname: 'blog.own', upstreamPort: 4000, tls: false, enabled: false },
];

function renderTable(overrides = {}) {
  const handlers = {
    onToggle: vi.fn().mockResolvedValue(undefined),
    onSetTls: vi.fn().mockResolvedValue(undefined),
    onUpdatePort: vi.fn().mockResolvedValue(undefined),
    onRemove: vi.fn().mockResolvedValue(undefined),
    ...overrides,
  };

  render(<DomainTable busy={false} mappings={MAPPINGS} {...handlers} />);

  return handlers;
}

describe('DomainTable cards', () => {
  it('renders one card per domain in a grid', () => {
    renderTable();

    expect(screen.getByText('shop.own')).toBeInTheDocument();
    expect(screen.getByText('blog.own')).toBeInTheDocument();
  });

  it('toggles enablement from the card switch', () => {
    const { onToggle } = renderTable();

    fireEvent.click(screen.getByRole('switch', { name: 'Enable shop.own' }));

    expect(onToggle).toHaveBeenCalledWith(MAPPINGS[0]);
  });

  it('toggles TLS from the card switch', () => {
    const { onSetTls } = renderTable();

    fireEvent.click(screen.getByRole('switch', { name: 'HTTPS for shop.own' }));

    expect(onSetTls).toHaveBeenCalledWith(MAPPINGS[0]);
  });

  it('confirms before deleting', () => {
    const { onRemove } = renderTable();

    fireEvent.click(screen.getByRole('button', { name: 'Delete shop.own' }));

    expect(onRemove).not.toHaveBeenCalled();
    expect(screen.getByRole('button', { name: 'Confirm delete' })).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Confirm delete' }));

    expect(onRemove).toHaveBeenCalledWith(MAPPINGS[0]);
  });
});
