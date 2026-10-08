// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Locks the Home landing for regular users, the logo's way back to it, and the retired Coach and Groups tabs
// ABOUTME: A stale #insights, #my-coaches or #groups hash lands on Home; the athlete's nav has neither

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, act, waitFor, within } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import Dashboard from '../Dashboard';

vi.mock('../dashboard/index', () => ({
  ConversationList: () => null,
  useUnreadConversationsCount: () => 0,
  usePendingUsersCount: () => 0,
  useStoreStatsPendingCount: () => 0,
}));

vi.mock('../../hooks/useNotifications', () => ({
  useUnreadCount: () => ({ unreadCount: 0, isLoading: false }),
}));

// Home is ChatTab's personal layout; the Groups tab is its shell.
vi.mock('../ChatTab', () => ({
  default: ({ layout }: { layout?: string }) =>
    layout === 'personal' ? (
      <div data-testid="home-tab">Home surface</div>
    ) : (
      <div data-testid="chat-tab">Groups surface</div>
    ),
}));

vi.mock('../StoreScreen', () => ({
  default: () => <div data-testid="discover-tab">Discover surface</div>,
}));

vi.mock('../home/Home', () => ({ HomeBriefing: () => null }));
vi.mock('../home/TodayPeek', () => ({ TodayPeek: () => null }));
// Home opens on the latest personal thread; these specs assert nothing about
// which one, and no #chat link here names a thread the list knows.
vi.mock('../../hooks/useConversationList', () => ({
  useLatestPersonalConversation: () => ({ id: null, isLoading: false }),
  useConversationScope: () => null,
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

describe('Dashboard landing — regular user', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.history.replaceState(null, '', '/');
  });

  it('lands on Home with no hash', async () => {
    await act(async () => {
      renderDashboard();
    });

    expect(await screen.findByTestId('home-tab')).toBeInTheDocument();
    expect(screen.queryByTestId('chat-tab')).toBeNull();
    expect(window.location.hash).toBe('#home');
  });

  it('still opens chat for a #chat deep link', async () => {
    window.history.replaceState(null, '', '/#chat');
    await act(async () => {
      renderDashboard();
    });

    expect(await screen.findByTestId('chat-tab')).toBeInTheDocument();
    expect(window.location.hash).toBe('#chat');
  });

  it('takes the athlete Home from the rail logo', async () => {
    window.history.replaceState(null, '', '/#discover');
    await act(async () => {
      renderDashboard();
    });
    expect(await screen.findByTestId('discover-tab')).toBeInTheDocument();

    await act(async () => {
      screen.getByTestId('rail-logo-home').click();
    });

    expect(await screen.findByTestId('home-tab')).toBeInTheDocument();
    expect(screen.getByTestId('rail-logo-home')).toHaveAccessibleName('Home');
    await waitFor(() => expect(window.location.hash).toBe('#home'));
  });

  it.each(['#insights', '#insights/friends', '#my-coaches', '#groups', '#groups/group-1'])(
    'resolves a stale %s deep link to Home on first load',
    async (hash) => {
      window.history.replaceState(null, '', `/${hash}`);

      await act(async () => {
        renderDashboard();
      });

      expect(await screen.findByTestId('home-tab')).toBeInTheDocument();
      expect(screen.queryByTestId('discover-tab')).toBeNull();
      // The retired hash is rewritten, so a reload does not replay it.
      expect(window.location.hash).toBe('#home');
    },
  );

  it('resolves a stale #my-coaches hash typed after load to Home', async () => {
    window.history.replaceState(null, '', '/#discover');
    await act(async () => {
      renderDashboard();
    });
    expect(await screen.findByTestId('discover-tab')).toBeInTheDocument();

    await act(async () => {
      window.location.hash = '#my-coaches';
    });

    expect(await screen.findByTestId('home-tab')).toBeInTheDocument();
    await waitFor(() => expect(window.location.hash).toBe('#home'));
  });

  // Notifications left the rail for the bell at the top right of each
  // surface (carnet#820).
  it('offers exactly Home, Groups and Discover in the rail — providers live under Settings', async () => {
    await act(async () => {
      renderDashboard();
    });

    const nav = screen.getByRole('list');
    const labels = within(nav)
      .getAllByRole('button')
      .map((button) => button.textContent?.trim());
    expect(labels).toEqual(['Home', 'Groups', 'Discover']);
  });

  it('resolves a stale #groups hash typed after load to Home', async () => {
    window.history.replaceState(null, '', '/#discover');
    await act(async () => {
      renderDashboard();
    });
    expect(await screen.findByTestId('discover-tab')).toBeInTheDocument();

    await act(async () => {
      window.location.hash = '#groups/group-1';
    });

    expect(await screen.findByTestId('home-tab')).toBeInTheDocument();
    await waitFor(() => expect(window.location.hash).toBe('#home'));
  });
});
