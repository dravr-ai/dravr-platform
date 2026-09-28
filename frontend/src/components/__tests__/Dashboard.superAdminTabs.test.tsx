// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Locks the super-admin-only tabs behind the role that the server serves them to
// ABOUTME: A plain admin's #impersonation-log or #admin-tokens lands on Users; a super admin opens the log

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, act, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import Dashboard from '../Dashboard';

const auth = vi.hoisted(() => ({ role: 'admin' }));

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
  default: () => <div data-testid="users-tab">Users surface</div>,
}));

vi.mock('../ImpersonationLogTab', () => ({
  default: () => <div data-testid="impersonation-log-tab">Impersonation log surface</div>,
}));

vi.mock('../ConnectProviderBanner', () => ({
  ConnectProviderBanner: () => null,
}));

vi.mock('../../hooks/useAuth', () => ({
  useAuth: () => ({
    user: { id: 'op-1', email: 'op@acme.com', display_name: 'Operator', role: auth.role },
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

describe('Dashboard super-admin tabs', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.history.replaceState(null, '', '/');
  });

  it.each(['#impersonation-log', '#admin-tokens'])(
    'resolves a plain admin %s deep link to Users on first load',
    async (hash) => {
      auth.role = 'admin';
      window.history.replaceState(null, '', `/${hash}`);

      await act(async () => {
        renderDashboard();
      });

      expect(await screen.findByTestId('users-tab')).toBeInTheDocument();
      expect(screen.queryByTestId('impersonation-log-tab')).toBeNull();
      expect(window.location.hash).toBe('#users');
    },
  );

  it('resolves a plain admin #impersonation-log typed after load to Users', async () => {
    auth.role = 'admin';
    await act(async () => {
      renderDashboard();
    });
    expect(await screen.findByTestId('users-tab')).toBeInTheDocument();

    await act(async () => {
      window.location.hash = '#impersonation-log';
    });

    await waitFor(() => expect(window.location.hash).toBe('#users'));
    expect(screen.queryByTestId('impersonation-log-tab')).toBeNull();
  });

  it('opens the impersonation log for a super admin', async () => {
    auth.role = 'super_admin';
    window.history.replaceState(null, '', '/#impersonation-log');

    await act(async () => {
      renderDashboard();
    });

    expect(await screen.findByTestId('impersonation-log-tab')).toBeInTheDocument();
    expect(window.location.hash).toBe('#impersonation-log');
  });
});
