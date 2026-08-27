// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the empty chat pane — one line, the host's "+", and the Commands button
// ABOUTME: Pins that the slash hint is the shared constant, not a second copy of the sentence

import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { SLASH_HINT } from '@pierre/shared-constants';
import ChatEmptyState from '../ChatEmptyState';

function renderEmptyState(overrides: { disabled?: boolean } = {}) {
  const onOpenCommands = vi.fn();
  render(
    <ChatEmptyState
      compose={<button type="button">Compose</button>}
      onOpenCommands={onOpenCommands}
      disabled={overrides.disabled}
    />,
  );
  return { onOpenCommands };
}

describe('ChatEmptyState', () => {
  it('shows one line, the host compose slot and the Commands button', () => {
    renderEmptyState();

    expect(screen.getByTestId('chat-empty-state')).toBeInTheDocument();
    expect(screen.getByText('Pick a chat, or start one')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Compose' })).toBeInTheDocument();
    expect(screen.getByTestId('chat-empty-commands')).toHaveTextContent('Commands');
  });

  it('teaches the "/" and "@" grammar with the shared hint', () => {
    renderEmptyState();
    expect(screen.getByText(SLASH_HINT)).toBeInTheDocument();
  });

  it('asks the host to open a thread with the palette when Commands is pressed', async () => {
    const user = userEvent.setup();
    const { onOpenCommands } = renderEmptyState();

    await user.click(screen.getByTestId('chat-empty-commands'));

    expect(onOpenCommands).toHaveBeenCalledTimes(1);
  });

  it('disables Commands while a conversation is being created', () => {
    renderEmptyState({ disabled: true });
    expect(screen.getByTestId('chat-empty-commands')).toBeDisabled();
  });
});
