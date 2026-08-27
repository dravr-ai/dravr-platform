// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Locks the /groups/join/:code landing now that the Groups tab is gone — chat plus one command
// ABOUTME: The join is the same /group join CODE turn a Telegram or WhatsApp member sends

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, act, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import Dashboard from '../Dashboard';
import type { PendingComposerAction } from '../ChatTab';

const chatTabProps = vi.fn();

vi.mock('../dashboard/index', () => ({
  ConversationList: () => null,
  useUnreadConversationsCount: () => 0,
  usePendingUsersCount: () => 0,
  useStoreStatsPendingCount: () => 0,
}));

vi.mock('../../hooks/useNotifications', () => ({
  useUnreadCount: () => ({ unreadCount: 0, isLoading: false }),
}));

vi.mock('../ChatTab', () => ({
  default: (props: {
    pendingComposerAction?: PendingComposerAction | null;
    onPendingComposerActionConsumed?: () => void;
  }) => {
    chatTabProps(props.pendingComposerAction);
    return (
      <div data-testid="chat-tab">
        <button type="button" onClick={() => props.onPendingComposerActionConsumed?.()}>
          consume
        </button>
      </div>
    );
  },
}));

vi.mock('../StoreScreen', () => ({
  default: () => <div data-testid="discover-tab">Discover surface</div>,
}));

vi.mock('../ConnectProviderBanner', () => ({ ConnectProviderBanner: () => null }));

vi.mock('../../hooks/useAuth', () => ({
  useAuth: () => ({
    user: { id: 'u-1', email: 'alice@acme.com', display_name: 'Alice', role: 'user' },
    logout: vi.fn(),
    isAuthenticated: true,
    isLoading: false,
  }),
}));

vi.mock('../../services/api', () => ({}));
vi.mock('../../services/analytics', () => ({ track: vi.fn() }));

function renderDashboard(pendingInviteCode: string | null, onInviteCodeConsumed = vi.fn()) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <Dashboard
        pendingInviteCode={pendingInviteCode}
        onInviteCodeConsumed={onInviteCodeConsumed}
      />
    </QueryClientProvider>,
  );
}

describe('Dashboard invite deep link', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.history.replaceState(null, '', '/#discover');
  });

  it('lands on chat and hands the chat surface exactly one /group join turn', async () => {
    await act(async () => {
      renderDashboard('WELCOME2026');
    });

    expect(await screen.findByTestId('chat-tab')).toBeInTheDocument();
    await waitFor(() => expect(window.location.hash).toBe('#chat'));

    const actions = chatTabProps.mock.calls
      .map(([action]) => action)
      .filter((action): action is PendingComposerAction => Boolean(action));
    // The same action object every render, never a second queued turn.
    expect(new Set(actions.map((a) => `${a.kind}:${a.text}`))).toEqual(
      new Set(['send:/group join WELCOME2026']),
    );
  });

  it('clears the action and tells the shell once the turn has gone out', async () => {
    const onInviteCodeConsumed = vi.fn();
    await act(async () => {
      renderDashboard('WELCOME2026', onInviteCodeConsumed);
    });

    await screen.findByTestId('chat-tab');
    await act(async () => {
      screen.getByRole('button', { name: 'consume' }).click();
    });

    expect(onInviteCodeConsumed).toHaveBeenCalledTimes(1);
    const last = chatTabProps.mock.calls[chatTabProps.mock.calls.length - 1][0];
    expect(last).toBeNull();
  });

  it('queues nothing without an invite code', async () => {
    await act(async () => {
      renderDashboard(null);
    });

    expect(await screen.findByTestId('discover-tab')).toBeInTheDocument();
    expect(chatTabProps).not.toHaveBeenCalled();
  });
});
