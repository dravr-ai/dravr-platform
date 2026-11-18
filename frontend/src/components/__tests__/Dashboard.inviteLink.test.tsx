// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Locks where a group invite lands once the server has joined it: the Groups tab, on the room's conversation
// ABOUTME: The join happened on the server (carnet#1648); a room it could not file opens on the Groups list

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, act, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import Dashboard from '../Dashboard';
import type { JoinedGroupRoom } from '../../hooks/useHeldGroupInvite';

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

// A group room stays on the Groups tab; nothing here is a personal thread.
vi.mock('../../hooks/useConversationList', () => ({
  usePersonalConversation: () => ({ id: null, isLoading: false }),
  useConversationScope: () => 'group',
}));

vi.mock('../ChatTab', () => ({
  default: (props: { selectedConversation: string | null }) => {
    chatTabProps({ selected: props.selectedConversation });
    return <div data-testid="chat-tab" />;
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

vi.mock('../../services/api', () => ({
  // Read by the shared hook bindings at import; this spec asserts nothing they fetch.
  featureFlagsApi: {},
}));
vi.mock('../../services/analytics', () => ({ track: vi.fn() }));

function renderDashboard(joinedRoom: JoinedGroupRoom | null, onJoinedRoomOpened = vi.fn()) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <Dashboard joinedRoom={joinedRoom} onJoinedRoomOpened={onJoinedRoomOpened} />
    </QueryClientProvider>,
  );
}

describe('Dashboard after a held invite joined', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.history.replaceState(null, '', '/#discover');
  });

  it('opens the room the join filed, where the join notice waits', async () => {
    const onJoinedRoomOpened = vi.fn();
    await act(async () => {
      renderDashboard({ conversationId: 'conv-group-1' }, onJoinedRoomOpened);
    });

    expect(await screen.findByTestId('chat-tab')).toBeInTheDocument();
    await waitFor(() => expect(window.location.hash).toBe('#chat/conv-group-1'));
    expect(chatTabProps).toHaveBeenLastCalledWith({ selected: 'conv-group-1' });
    expect(onJoinedRoomOpened).toHaveBeenCalled();
  });

  it('opens the Groups list when the server could not file the room', async () => {
    await act(async () => {
      renderDashboard({ conversationId: null });
    });

    expect(await screen.findByTestId('chat-tab')).toBeInTheDocument();
    await waitFor(() => expect(window.location.hash).toBe('#chat'));
    expect(chatTabProps).toHaveBeenLastCalledWith({ selected: null });
  });

  it('stays where it opened without a joined room', async () => {
    await act(async () => {
      renderDashboard(null);
    });

    expect(await screen.findByTestId('discover-tab')).toBeInTheDocument();
    expect(chatTabProps).not.toHaveBeenCalled();
  });
});
