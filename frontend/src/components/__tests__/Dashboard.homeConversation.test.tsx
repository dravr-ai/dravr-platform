// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Locks Home as the athlete's own conversation — it opens on the latest personal thread, or the one its hash names
// ABOUTME: A #chat link to a personal thread moves to Home once the list says so; a room stays in the Groups tab

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, act, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import Dashboard from '../Dashboard';

const chatTabProps = vi.fn();
const latest = vi.hoisted(() => ({ id: 'conv-latest' as string | null, isLoading: false }));
const scopes = vi.hoisted((): Record<string, 'groups' | 'personal'> => ({}));

vi.mock('../dashboard/index', () => ({
  ConversationList: () => null,
  useUnreadConversationsCount: () => 0,
  usePendingUsersCount: () => 0,
  useStoreStatsPendingCount: () => 0,
}));

vi.mock('../../hooks/useConversationList', () => ({
  useLatestPersonalConversation: () => latest,
  // `undefined` would be a list still loading; these answers are all read.
  useConversationScope: (id: string | null) => (id === null ? null : (scopes[id] ?? null)),
}));

vi.mock('../../hooks/useNotifications', () => ({
  useUnreadCount: () => ({ unreadCount: 0, isLoading: false }),
}));

vi.mock('../ChatTab', () => ({
  default: (props: { layout?: string; selectedConversation: string | null; resolving?: boolean }) => {
    chatTabProps({ layout: props.layout ?? 'shell', selected: props.selectedConversation, resolving: props.resolving ?? false });
    return <div data-testid={props.layout === 'personal' ? 'home-tab' : 'chat-tab'} />;
  },
}));

vi.mock('../home/Home', () => ({ HomeBriefing: () => null }));
vi.mock('../home/TodayPeek', () => ({ TodayPeek: () => null }));
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

function lastChatTab() {
  return chatTabProps.mock.calls[chatTabProps.mock.calls.length - 1][0];
}

describe('Dashboard — Home is the athlete’s own conversation', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.history.replaceState(null, '', '/');
    latest.id = 'conv-latest';
    latest.isLoading = false;
    for (const key of Object.keys(scopes)) delete scopes[key];
  });

  it('opens on the latest personal thread, with the plain #home hash', async () => {
    await act(async () => {
      renderDashboard();
    });

    expect(await screen.findByTestId('home-tab')).toBeInTheDocument();
    expect(lastChatTab()).toEqual({ layout: 'personal', selected: 'conv-latest', resolving: false });
    expect(window.location.hash).toBe('#home');
  });

  it('says it is still resolving while the list has not answered', async () => {
    latest.id = null;
    latest.isLoading = true;
    await act(async () => {
      renderDashboard();
    });

    await screen.findByTestId('home-tab');
    expect(lastChatTab()).toEqual({ layout: 'personal', selected: null, resolving: true });
  });

  it('opens the thread a #home/chat hash names, and keeps it in the hash', async () => {
    window.history.replaceState(null, '', '/#home/chat/conv-older');
    await act(async () => {
      renderDashboard();
    });

    await screen.findByTestId('home-tab');
    expect(lastChatTab()).toEqual({ layout: 'personal', selected: 'conv-older', resolving: false });
    expect(window.location.hash).toBe('#home/chat/conv-older');
  });

  it('moves a #chat link to a personal thread onto Home', async () => {
    scopes['conv-telegram'] = 'personal';
    window.history.replaceState(null, '', '/#chat/conv-telegram');
    await act(async () => {
      renderDashboard();
    });

    expect(await screen.findByTestId('home-tab')).toBeInTheDocument();
    expect(lastChatTab()).toEqual({ layout: 'personal', selected: 'conv-telegram', resolving: false });
    await waitFor(() => expect(window.location.hash).toBe('#home/chat/conv-telegram'));
  });

  it('leaves a #chat link to a room in the Groups tab', async () => {
    scopes['room-1'] = 'groups';
    window.history.replaceState(null, '', '/#chat/room-1');
    await act(async () => {
      renderDashboard();
    });

    expect(await screen.findByTestId('chat-tab')).toBeInTheDocument();
    expect(lastChatTab()).toEqual({ layout: 'shell', selected: 'room-1', resolving: false });
    expect(window.location.hash).toBe('#chat/room-1');
  });

  it('returns Home to the latest thread from the rail logo', async () => {
    window.history.replaceState(null, '', '/#home/chat/conv-older');
    await act(async () => {
      renderDashboard();
    });
    await screen.findByTestId('home-tab');

    await act(async () => {
      screen.getByTestId('rail-logo-home').click();
    });

    expect(lastChatTab()).toEqual({ layout: 'personal', selected: 'conv-latest', resolving: false });
    await waitFor(() => expect(window.location.hash).toBe('#home'));
  });
});
