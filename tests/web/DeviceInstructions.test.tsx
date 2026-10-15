// @vitest-environment jsdom
import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { DeviceInstructions } from '../../src/web/features/setup/DeviceInstructions';

describe('DeviceInstructions', () => {
  it('explains DNS and certificate trust requirements', () => {
    render(<DeviceInstructions lanAddress="192.168.1.20" />);

    expect(screen.getByText('192.168.1.20')).toBeInTheDocument();
    expect(screen.getByText(/full certificate trust/i)).toBeInTheDocument();
  });
});
