// @vitest-environment jsdom
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { Modal } from '../../src/web/features/modal/Modal';

describe('Modal', () => {
  it('renders nothing when closed', () => {
    const { container } = render(<Modal open={false} onClose={() => undefined} label="Edit"><p>body</p></Modal>);

    expect(container.firstChild).toBeNull();
  });

  it('renders the dialog with a close button', () => {
    render(<Modal open onClose={() => undefined} label="Edit domain"><p>body</p></Modal>);

    expect(screen.getByRole('dialog', { name: 'Edit domain' })).toBeInTheDocument();
  });

  it('closes on close button, overlay click, and Escape', () => {
    const onClose = vi.fn();
    const { rerender } = render(<Modal open onClose={onClose} label="Edit domain"><p>body</p></Modal>);

    fireEvent.click(screen.getByRole('button', { name: 'Close' }));

    expect(onClose).toHaveBeenCalledTimes(1);

    rerender(<Modal open onClose={onClose} label="Edit domain"><p>body</p></Modal>);
    fireEvent.click(screen.getByTestId('modal-overlay'));

    expect(onClose).toHaveBeenCalledTimes(2);

    rerender(<Modal open onClose={onClose} label="Edit domain"><p>body</p></Modal>);
    fireEvent.keyDown(document, { key: 'Escape' });

    expect(onClose).toHaveBeenCalledTimes(3);
  });

  it('traps focus inside the dialog', () => {
    const onClose = vi.fn();

    render(<Modal open onClose={onClose} label="Edit domain"><button type="button">First</button><button type="button">Last</button></Modal>);

    const buttons = screen.getAllByRole('button');

    expect(document.activeElement).toBe(buttons[0]);

    (buttons.at(-1) as HTMLElement).focus();
    fireEvent.keyDown(document, { key: 'Tab' });

    expect(document.activeElement).toBe(buttons[0]);

    buttons[0].focus();
    fireEvent.keyDown(document, { key: 'Tab', shiftKey: true });

    expect(document.activeElement).toBe(buttons.at(-1));
  });
});
