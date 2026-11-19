// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the shell an admin's sign-in mode draws (carnet#1631): the athlete app in the athlete mode, the console otherwise
// ABOUTME: The switch is offered to an admin in both shells, clears the route, and never shows to an athlete

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, act } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import Dashboard from '../Dashboard';

const auth = vi.hoisted(() => ({ role: 'admin', viewMode: null as 'admin' | 'athlete' | null }));
const chooseViewMode = vi.hoisted(() => vi.fn());
// The role of the signed-in account in each render of a mocked admin pane.
const drawn = vi.hoisted(() => ({ as: [] as string[] }));

vi.mock('../dashboard/index', () => ({
  ConversationList: () => null,
  useUnreadConversationsCount: () => 0,
  usePendingUsersCount: () => 0,
  useStoreStatsPendingCount: () => 0,
}));

vi.mock('../../hooks/useNotifications', () => ({
  useUnreadCount: () => ({ unreadCount: 0, isLoading: false }),
}));

vi.mock('../UserManagement', () => ({
  default: () => {
    drawn.as.push(auth.role);
    return <div data-testid="users-tab">Users surface</div>;
  },
}));

vi.mock('../ImpersonationLogTab', () => ({
  default: () => {
    drawn.as.push(auth.role);
    return <div data-testid="impersonation-log-tab">Impersonation log surface</div>;
  },
}));

vi.mock('../UnifiedConnections', () => ({
  default: () => {
    drawn.as.push(auth.role);
    return <div data-testid="connections-tab">Token surface</div>;
  },
}));

// Where a user without the admin role lands: Home is PersonalChat.
vi.mock('../home/PersonalChat', () => ({
  default: () => <div data-testid="home-tab">Home surface</div>,
}));
vi.mock('../home/Home', () => ({ HomeBriefing: () => null }));
vi.mock('../home/TodayPeek', () => ({ TodayPeek: () => null }));
vi.mock('../../hooks/useConversationList', () => ({
  usePersonalConversation: () => ({ id: null, isLoading: false }),
  useConversationScope: () => null,
}));

// The hash is what these cases pin; a pane's own contents have their own specs.
vi.mock('../UserSettings', () => ({
  default: ({ initialTab }: { initialTab?: string }) => <div data-testid="user-settings">{initialTab ?? 'default'}</div>,
}));

vi.mock('../ConnectProviderBanner', () => ({
  ConnectProviderBanner: () => null,
}));

vi.mock('../../hooks/useAuth', () => ({
  useAuth: () => ({
    user: { id: 'admin-1', email: 'admin@acme.com', display_name: 'Admin', role: auth.role },
    viewMode: auth.viewMode,
    chooseViewMode,
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

describe('Dashboard view mode', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    drawn.as = [];
    auth.role = 'admin';
    auth.viewMode = null;
    window.history.replaceState(null, '', '/');
  });

  it('draws the athlete app for an admin in the athlete mode: Home, the rail, no console', async () => {
    auth.viewMode = 'athlete';
    await act(async () => {
      renderDashboard();
    });

    expect(await screen.findByTestId('home-tab')).toBeInTheDocument();
    expect(screen.getByTestId('icon-rail')).toBeInTheDocument();
    expect(screen.queryByTestId('users-tab')).toBeNull();
    expect(window.location.hash).toBe('#home');
  });

  it('resolves an admin tab typed in the athlete mode to Home, as for any athlete', async () => {
    auth.viewMode = 'athlete';
    window.history.replaceState(null, '', '/#users');
    await act(async () => {
      renderDashboard();
    });

    expect(await screen.findByTestId('home-tab')).toBeInTheDocument();
    expect(drawn.as).toEqual([]);
    expect(window.location.hash).toBe('#home');
  });

  it('opens the console for an admin in the admin mode', async () => {
    auth.viewMode = 'admin';
    await act(async () => {
      renderDashboard();
    });

    expect(await screen.findByTestId('users-tab')).toBeInTheDocument();
    expect(screen.queryByTestId('icon-rail')).toBeNull();
  });

  it('offers the console a switch to the athlete app that clears the route first', async () => {
    auth.viewMode = 'admin';
    await act(async () => {
      renderDashboard();
    });
    expect(window.location.hash).toBe('#users');

    await userEvent.click(screen.getByTestId('switch-view-mode'));

    expect(chooseViewMode).toHaveBeenCalledWith('athlete');
    expect(window.location.hash).toBe('');
  });

  it('offers the athlete app a switch back to the console from the account menu', async () => {
    auth.viewMode = 'athlete';
    await act(async () => {
      renderDashboard();
    });

    await userEvent.click(screen.getByTestId('rail-account-menu'));
    await userEvent.click(screen.getByTestId('switch-view-mode'));

    expect(chooseViewMode).toHaveBeenCalledWith('admin');
    expect(window.location.hash).toBe('');
  });

  it('never offers an athlete a switch', async () => {
    auth.role = 'user';
    auth.viewMode = 'admin';
    await act(async () => {
      renderDashboard();
    });

    expect(await screen.findByTestId('home-tab')).toBeInTheDocument();
    await userEvent.click(screen.getByTestId('rail-account-menu'));
    expect(screen.queryByTestId('switch-view-mode')).toBeNull();
  });
});
