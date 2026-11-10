// @vitest-environment jsdom
import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { DomainMapping } from '../../src/shared/types';
import { DomainRow } from '../../src/web/features/domains/DomainRow';

const { openUrl } = vi.hoisted(() => ({ openUrl: vi.fn().mockResolvedValue(undefined) }));

vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl }));

const MAPPING: DomainMapping = { id: 'one', hostname: 'shop.own', upstreamPort: 3000, tls: true, enabled: true };

const HANDLERS = {
  onToggle: async () => undefined,
  onSetTls: async () => undefined,
  onUpdatePort: async () => undefined,
  onRemove: async () => undefined,
};

beforeEach(() => {
  openUrl.mockClear();
});

describe('DomainRow external link', () => {
  it('opens the domain in the system browser', () => {
    render(<DomainRow busy={false} mapping={MAPPING} {...HANDLERS} />);

    fireEvent.click(screen.getByRole('link', { name: 'shop.own' }));

    expect(openUrl).toHaveBeenCalledWith('https://shop.own');
  });
});
