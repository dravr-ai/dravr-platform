// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Locks Home's way into chat — a tap hands the chat surface one draft, in a fresh thread, never a sent turn
// ABOUTME: A thread that was open before the athlete went Home is closed first, so the draft never lands in it

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
    selectedConversation: string | null;
    pendingComposerAction?: PendingComposerAction | null;
  }) => {
    chatTabProps({ selected: props.selectedConversation, action: props.pendingComposerAction ?? null });
    return <div data-testid="chat-tab" />;
  },
}));

const DRAFT = 'Analyze my activity from Tuesday 23 September (Run)';

// Home's own rendering is covered by its tests; here it only has to hand the
// shell a draft the way a tapped activity row does.
vi.mock('../home/Home', () => ({
  default: ({ onOpenChatDraft }: { onOpenChatDraft: (text: string) => void }) => (
    <div data-testid="home-tab">
      <button type="button" onClick={() => onOpenChatDraft(DRAFT)}>
        tap activity
      </button>
    </div>
  ),
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

function renderDashboard() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <Dashboard />
    </QueryClientProvider>,
  );
}

describe('Dashboard — Home opens a chat draft', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.history.replaceState(null, '', '/');
  });

  it('switches to chat with exactly one draft action, not a send', async () => {
    await act(async () => {
      renderDashboard();
    });
    await act(async () => {
      (await screen.findByRole('button', { name: 'tap activity' })).click();
    });

    expect(await screen.findByTestId('chat-tab')).toBeInTheDocument();
    await waitFor(() => expect(window.location.hash).toBe('#chat'));
    const actions = chatTabProps.mock.calls
      .map(([props]) => props.action as PendingComposerAction | null)
      .filter((action): action is PendingComposerAction => action !== null);
    expect(actions[0]).toEqual({ kind: 'draft', text: DRAFT });
  });

  it('closes the thread that was open before going Home, so the draft starts a fresh one', async () => {
    window.history.replaceState(null, '', '/#chat/conv-open');
    await act(async () => {
      renderDashboard();
    });
    expect(await screen.findByTestId('chat-tab')).toBeInTheDocument();
    expect(chatTabProps).toHaveBeenLastCalledWith({ selected: 'conv-open', action: null });

    await act(async () => {
      screen.getByTestId('rail-logo-home').click();
    });
    await act(async () => {
      (await screen.findByRole('button', { name: 'tap activity' })).click();
    });

    await screen.findByTestId('chat-tab');
    expect(chatTabProps).toHaveBeenLastCalledWith({
      selected: null,
      action: { kind: 'draft', text: DRAFT },
    });
  });
});
