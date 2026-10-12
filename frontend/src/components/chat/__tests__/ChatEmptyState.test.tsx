// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the empty chat pane — one line, the host's "+", and the three use-case links ranked for the athlete
// ABOUTME: Pins that a link hands the host its starter and that the slash hint is the shared constant, not a second copy

import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { UseCaseStarter } from '@pierre/shared-types';
import ChatEmptyState from '../ChatEmptyState';

const STARTERS: UseCaseStarter[] = [
  { id: 'connect', label: 'Connect my watch', postback: 'uc:0:connect' },
  { id: 'about_me', label: 'Get to know me', postback: 'uc:1:about_me' },
  { id: 'zone2', label: 'What is Zone 2?', postback: 'uc:2:zone2' },
];

function renderEmptyState(
  overrides: { disabled?: boolean; providerStatus?: string | null; starters?: UseCaseStarter[] } = {},
) {
  const onStarter = vi.fn();
  render(
    <ChatEmptyState
      compose={<button type="button">Compose</button>}
      starters={overrides.starters ?? STARTERS}
      onStarter={onStarter}
      disabled={overrides.disabled}
      providerStatus={overrides.providerStatus}
    />,
  );
  return { onStarter };
}

describe('ChatEmptyState', () => {
  it('shows one line, the host compose slot and the three ranked starters as links, in rank order', () => {
    renderEmptyState();

    expect(screen.getByTestId('chat-empty-state')).toBeInTheDocument();
    expect(screen.getByText('Pick a chat, or start one')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Compose' })).toBeInTheDocument();
    const links = screen.getAllByTestId(/^chat-empty-starter-/);
    expect(links.map((link) => link.textContent)).toEqual([
      'Connect my watch',
      'Get to know me',
      'What is Zone 2?',
    ]);
  });

  it('no longer points at features: no Commands, Route, Discover or Connect link', () => {
    renderEmptyState();
    for (const testId of ['chat-empty-commands', 'chat-empty-route', 'chat-empty-discover', 'chat-empty-connect']) {
      expect(screen.queryByTestId(testId)).not.toBeInTheDocument();
    }
  });

  it('teaches the "/" and "@" grammar with the shared hint', () => {
    renderEmptyState();
    expect(
      screen.getByText('Type / for commands · @handle brings an agent in for one turn'),
    ).toBeInTheDocument();
  });

  it('hands the host the starter that was tapped, postback and label together', async () => {
    const user = userEvent.setup();
    const { onStarter } = renderEmptyState();

    await user.click(screen.getByTestId('chat-empty-starter-about_me'));

    expect(onStarter).toHaveBeenCalledExactlyOnceWith(STARTERS[1]);
  });

  it('disables the starters while a conversation is being created', () => {
    renderEmptyState({ disabled: true });
    for (const link of screen.getAllByTestId(/^chat-empty-starter-/)) {
      expect(link).toBeDisabled();
    }
  });

  it('draws no link while the starters load or when none fits, and keeps the hint', () => {
    renderEmptyState({ starters: [] });
    expect(screen.queryAllByTestId(/^chat-empty-starter-/)).toHaveLength(0);
    expect(screen.getByRole('button', { name: 'Compose' })).toBeInTheDocument();
    expect(
      screen.getByText('Type / for commands · @handle brings an agent in for one turn'),
    ).toBeInTheDocument();
  });

  it('names what the agent can see in the footer, and stays quiet until that is known', () => {
    const { unmount } = render(
      <ChatEmptyState
        compose={<button type="button">Compose</button>}
        starters={STARTERS}
        onStarter={vi.fn()}
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
});
