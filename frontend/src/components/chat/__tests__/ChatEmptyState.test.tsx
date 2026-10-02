// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the empty chat pane — one line, the host's "+", and the Commands button
// ABOUTME: Pins that the slash hint is the shared constant, not a second copy of the sentence

import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import ChatEmptyState from '../ChatEmptyState';

function renderEmptyState(
  overrides: { disabled?: boolean; providerStatus?: string | null } = {},
) {
  const onOpenCommands = vi.fn();
  render(
    <ChatEmptyState
      compose={<button type="button">Compose</button>}
      onOpenCommands={onOpenCommands}
      disabled={overrides.disabled}
      providerStatus={overrides.providerStatus}
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
    expect(
      screen.getByText('Type / for commands · @handle brings an agent in for one turn'),
    ).toBeInTheDocument();
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

  it('names what the agent can see in the footer, and stays quiet until that is known', () => {
    const { unmount } = render(
      <ChatEmptyState
        compose={<button type="button">Compose</button>}
        onOpenCommands={vi.fn()}
        providerStatus={null}
      />,
    );
    expect(screen.queryByTestId('chat-empty-provider-status')).not.toBeInTheDocument();
    unmount();

    renderEmptyState({ providerStatus: 'No provider connected' });
    expect(screen.getByTestId('chat-empty-provider-status')).toHaveTextContent(
      'No provider connected',
    );
  });
  it('suggests a route for today\'s session and asks the host to draft it', async () => {
    const user = userEvent.setup();
    const onSuggestRoute = vi.fn();
    render(
      <ChatEmptyState
        compose={<button type="button">Compose</button>}
        onOpenCommands={vi.fn()}
        onSuggestRoute={onSuggestRoute}
      />,
    );

    const route = screen.getByTestId('chat-empty-route');
    expect(route).toHaveTextContent("Route for today's session");
    await user.click(route);

    expect(onSuggestRoute).toHaveBeenCalledTimes(1);
  });

  it('leaves the suggestion out for a host that cannot draft it', () => {
    renderEmptyState();
    expect(screen.queryByTestId('chat-empty-route')).not.toBeInTheDocument();
  });
});
