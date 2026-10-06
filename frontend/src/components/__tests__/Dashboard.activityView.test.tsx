// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Locks the activity view's place in the shell — `#home/activity/<provider>/<id>`, opened from Home, left by Back
// ABOUTME: A deep link opens the view on the Home tab; a hash that names no activity is Home itself

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, act, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import Dashboard from '../Dashboard';

const viewProps = vi.fn();

vi.mock('../dashboard/index', () => ({
  ConversationList: () => null,
  useUnreadConversationsCount: () => 0,
  usePendingUsersCount: () => 0,
  useStoreStatsPendingCount: () => 0,
}));
vi.mock('../../hooks/useNotifications', () => ({
  useUnreadCount: () => ({ unreadCount: 0, isLoading: false }),
}));
// Home is ChatTab's personal layout; it draws the Today panel the shell hands it.
vi.mock('../ChatTab', () => ({
  default: (props: { layout?: string; today?: { panel: (draft: (text: string) => void, compact: boolean) => unknown } }) =>
    props.layout === 'personal' ? (
      <div data-testid="home-tab">{props.today?.panel(() => undefined, true) as never}</div>
    ) : (
      <div data-testid="chat-tab" />
    ),
}));
vi.mock('../ConnectProviderBanner', () => ({ ConnectProviderBanner: () => null }));

// Home's Today hands the shell the route a tapped activity row builds.
vi.mock('../home/Home', () => ({
  HomeBriefing: ({ onNavigate }: { onNavigate: (route: string) => void }) => (
    <button type="button" onClick={() => onNavigate('home/activity/strava/morning%20run')}>
      tap activity
    </button>
  ),
}));
vi.mock('../home/TodayPeek', () => ({ TodayPeek: () => null }));
// Home opens on the latest personal thread; these specs assert nothing about
// which one, and no #chat link here names a thread the list knows.
vi.mock('../../hooks/useConversationList', () => ({
  useLatestPersonalConversation: () => ({ id: null, isLoading: false }),
  useConversationScope: () => null,
}));

vi.mock('../activity/ActivityView', () => ({
  default: (props: { activity: { provider: string; id: string }; onBack: () => void }) => {
    viewProps(props.activity);
    return (
      <div data-testid="activity-view">
        <button type="button" onClick={props.onBack}>
          back
        </button>
      </div>
    );
  },
}));

vi.mock('../../hooks/useAuth', () => ({
  useAuth: () => ({
    user: { id: 'u-1', email: 'alice@acme.com', display_name: 'Alice', role: 'user' },
    logout: vi.fn(),
    isAuthenticated: true,
    isLoading: false,
  }),
}));
vi.mock('../../services/api', () => ({ featureFlagsApi: {} }));
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

describe('Dashboard — the activity view', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.history.replaceState(null, '', '/');
  });

  it('opens the tapped activity over Home, and Back returns to Home', async () => {
    window.history.replaceState(null, '', '/#home');
    await act(async () => {
      renderDashboard();
    });
    await act(async () => {
      (await screen.findByRole('button', { name: 'tap activity' })).click();
    });

    expect(await screen.findByTestId('activity-view')).toBeInTheDocument();
    expect(viewProps).toHaveBeenLastCalledWith({ provider: 'strava', id: 'morning run' });
    await waitFor(() => expect(window.location.hash).toBe('#home/activity/strava/morning%20run'));
    expect(screen.queryByTestId('home-tab')).toBeNull();

    await act(async () => {
      screen.getByRole('button', { name: 'back' }).click();
    });
    expect(await screen.findByTestId('home-tab')).toBeInTheDocument();
    await waitFor(() => expect(window.location.hash).toBe('#home'));
  });

  it('opens a deep-linked activity on the Home tab', async () => {
    window.history.replaceState(null, '', '/#home/activity/garmin/12345');
    await act(async () => {
      renderDashboard();
    });

    expect(await screen.findByTestId('activity-view')).toBeInTheDocument();
    expect(viewProps).toHaveBeenLastCalledWith({ provider: 'garmin', id: '12345' });
  });

  it('reads a Home hash that names no activity as Home itself', async () => {
    window.history.replaceState(null, '', '/#home/activity/garmin');
    await act(async () => {
      renderDashboard();
    });

    expect(await screen.findByTestId('home-tab')).toBeInTheDocument();
    expect(screen.queryByTestId('activity-view')).toBeNull();
  });
});
