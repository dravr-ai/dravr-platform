// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the Sheet — a dialog named by its title that Escape, the close button and the scrim all close
// ABOUTME: Its extra header actions sit beside the close button

import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Sheet } from '../Sheet';

function renderSheet(onClose = vi.fn()) {
  render(
    <Sheet side="bottom" title="Today" onClose={onClose} actions={<button type="button">New</button>}>
      <p>today’s session</p>
    </Sheet>,
  );
  return onClose;
}

describe('Sheet', () => {
  it('is a modal dialog named by its title, with its actions and content', () => {
    renderSheet();
    const dialog = screen.getByRole('dialog', { name: 'Today' });
    expect(dialog).toHaveAttribute('aria-modal', 'true');
    expect(screen.getByRole('button', { name: 'New' })).toBeInTheDocument();
    expect(screen.getByText('today’s session')).toBeInTheDocument();
  });

  it('closes on Escape', () => {
    const onClose = renderSheet();
    fireEvent.keyDown(document, { key: 'Escape' });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('closes from its close button and from the scrim', async () => {
    const user = userEvent.setup();
    const onClose = renderSheet();
    await user.click(screen.getByRole('button', { name: 'Close' }));
    expect(onClose).toHaveBeenCalledTimes(1);

    const scrim = document.querySelector('.bg-scrim\\/60');
    if (!scrim) throw new Error('no scrim');
    await user.click(scrim);
    expect(onClose).toHaveBeenCalledTimes(2);
  });
});
