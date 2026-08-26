// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Locks the chat-first landing for regular users and the retirement of the user Coach tab
// ABOUTME: A stale #insights or #my-coaches hash lands on chat; Discover, not Coaches, is the athlete's nav

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, act, waitFor, within } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import Dashboard from '../Dashboard';

vi.mock('../dashboard/index', () => ({
  ConversationsPanel: () => null,
  usePendingUsersCount: () => 0,
  useStoreStatsPendingCount: () => 0,
}));

vi.mock('../../hooks/useNotifications', () => ({
  useUnreadCount: () => ({ unreadCount: 0, isLoading: false }),
}));

vi.mock('../ChatTab', () => ({
  default: () => <div data-testid="chat-tab">Chat surface</div>,
}));

vi.mock('../StoreScreen', () => ({
  default: () => <div data-testid="discover-tab">Discover surface</div>,
}));

vi.mock('../ConnectProviderBanner', () => ({
  ConnectProviderBanner: () => null,
}));

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

describe('Dashboard landing — regular user', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.history.replaceState(null, '', '/');
  });

  it('lands on chat with no hash', async () => {
    await act(async () => {
      renderDashboard();
    });

    expect(await screen.findByTestId('chat-tab')).toBeInTheDocument();
    expect(window.location.hash).toBe('#chat');
  });

  it.each(['#insights', '#insights/friends', '#my-coaches'])(
    'resolves a stale %s deep link to chat on first load',
    async (hash) => {
      window.history.replaceState(null, '', `/${hash}`);

      await act(async () => {
        renderDashboard();
      });

      expect(await screen.findByTestId('chat-tab')).toBeInTheDocument();
      expect(screen.queryByTestId('discover-tab')).toBeNull();
      // The retired hash is rewritten, so a reload does not replay it.
      expect(window.location.hash).toBe('#chat');
    },
  );

  it('resolves a stale #my-coaches hash typed after load to chat', async () => {
    window.history.replaceState(null, '', '/#discover');
    await act(async () => {
      renderDashboard();
    });
    expect(await screen.findByTestId('discover-tab')).toBeInTheDocument();

    await act(async () => {
      window.location.hash = '#my-coaches';
    });

    expect(await screen.findByTestId('chat-tab')).toBeInTheDocument();
    await waitFor(() => expect(window.location.hash).toBe('#chat'));
  });

  it('offers Chat, Discover and Groups in the sidebar, and no Coaches tab', async () => {
    await act(async () => {
      renderDashboard();
    });

    const nav = screen.getByRole('list');
    expect(within(nav).getByRole('button', { name: 'Chat' })).toBeInTheDocument();
    expect(within(nav).getByRole('button', { name: 'Discover' })).toBeInTheDocument();
    expect(within(nav).getByRole('button', { name: 'Groups' })).toBeInTheDocument();
    expect(within(nav).queryByRole('button', { name: 'Coaches' })).toBeNull();
  });
});
