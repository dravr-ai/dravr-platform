// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A deep link to a tab whose feature gate is off lands on the role default, never a blank pane
// ABOUTME: With billing gated off, #billing and #usage would otherwise draw nothing under an empty heading

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, act, waitFor } from '@testing-library/react';
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

vi.mock('../UserManagement', () => ({
  default: () => <div data-testid="users-tab">Users surface</div>,
}));

vi.mock('../ConnectProviderBanner', () => ({
  ConnectProviderBanner: () => null,
}));

vi.mock('../../hooks/useAuth', () => ({
  useAuth: () => ({
    user: { id: 'op-1', email: 'op@acme.com', display_name: 'Operator', role: 'admin' },
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

describe('Dashboard gated tabs', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.history.replaceState(null, '', '/');
  });

  it.each(['#billing', '#usage'])('resolves a %s deep link to Users and names the page', async (hash) => {
    window.history.replaceState(null, '', `/${hash}`);

    await act(async () => {
      renderDashboard();
    });

    expect(await screen.findByTestId('users-tab')).toBeInTheDocument();
    expect(screen.getByRole('heading', { level: 1 })).not.toHaveTextContent(/^$/);
    await waitFor(() => expect(window.location.hash).toBe('#users'));
  });

  it('resolves a #billing hash typed after load to Users', async () => {
    await act(async () => {
      renderDashboard();
    });
    expect(await screen.findByTestId('users-tab')).toBeInTheDocument();

    await act(async () => {
      window.location.hash = '#billing';
    });

    await waitFor(() => expect(window.location.hash).toBe('#users'));
    expect(screen.getByTestId('users-tab')).toBeInTheDocument();
  });
});
