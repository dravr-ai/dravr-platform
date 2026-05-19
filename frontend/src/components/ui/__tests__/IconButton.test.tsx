// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, it, expect, vi } from 'vitest';
import { IconButton } from '../IconButton';

describe('IconButton', () => {
  it('renders with accessible label', () => {
    render(
      <IconButton aria-label="Close dialog">
        <svg data-testid="x-icon" />
      </IconButton>,
    );
    expect(screen.getByRole('button', { name: 'Close dialog' })).toBeInTheDocument();
    expect(screen.getByTestId('x-icon')).toBeInTheDocument();
  });

  it('enforces 44x44 minimum hit area at sm size', () => {
    render(
      <IconButton aria-label="Help" size="sm">
        <span />
      </IconButton>,
    );
    const btn = screen.getByRole('button', { name: 'Help' });
    expect(btn.className).toMatch(/min-w-\[44px\]/);
    expect(btn.className).toMatch(/min-h-\[44px\]/);
  });

  it('fires onClick', async () => {
    const onClick = vi.fn();
    render(
      <IconButton aria-label="Send" onClick={onClick}>
        <span />
      </IconButton>,
    );
    await userEvent.click(screen.getByRole('button', { name: 'Send' }));
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it('respects disabled state', () => {
    render(
      <IconButton aria-label="Disabled" disabled>
        <span />
      </IconButton>,
    );
    expect(screen.getByRole('button', { name: 'Disabled' })).toBeDisabled();
  });
});
