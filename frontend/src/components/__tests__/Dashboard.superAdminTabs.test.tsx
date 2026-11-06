// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Locks the super-admin-only tabs behind the role that the server serves them to
// ABOUTME: A plain admin's #impersonation-log lands on Users, a super admin opens it; moved and hidden hashes resolve

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

// The hash is what these cases pin; a pane's own contents have their own specs.
vi.mock('../UserSettings', () => ({
  default: ({ initialTab }: { initialTab?: string }) => <div data-testid="user-settings">{initialTab ?? 'default'}</div>,
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

  it.each(['#impersonation-log', '#harness-config'])(
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

  it('offers the settings tabs only to the role the server serves them to', async () => {
    auth.role = 'admin';
    const adminView = await act(async () => renderDashboard());
    expect(await screen.findByTestId('users-tab')).toBeInTheDocument();
    expect(screen.queryByText('shell.navHarnessConfig')).toBeNull();
    expect(screen.queryByText('Harness Config')).toBeNull();
    adminView.unmount();

    auth.role = 'super_admin';
    await act(async () => {
      renderDashboard();
    });
    await waitFor(() =>
      expect(screen.queryByText('Harness Config') ?? screen.queryByText('shell.navHarnessConfig')).not.toBeNull(),
    );
  });

  it.each(['admin', 'super_admin'])('opens a %s #admin-tokens bookmark on the one token screen', async (role) => {
    auth.role = role;
    window.history.replaceState(null, '', '/#admin-tokens');
    await act(async () => {
      renderDashboard();
    });
    await waitFor(() => expect(window.location.hash).toBe('#connections'));
  });

  it.each(['admin', 'super_admin'])('offers a %s one token entry and no inbox in the rail', async (role) => {
    auth.role = role;
    await act(async () => {
      renderDashboard();
    });
    expect(await screen.findByTestId('users-tab')).toBeInTheDocument();
    expect(screen.queryByText('Admin Tokens')).toBeNull();
    expect(screen.queryByText('shell.navAdminTokens')).toBeNull();
    expect(screen.queryByRole('button', { name: /^(Notifications|shell\.navNotifications)$/ })).toBeNull();
    expect(screen.getAllByText(/^(Service Tokens|API Tokens|shell\.navServiceTokens)$/).length).toBeGreaterThan(0);
  });

  it.each(['memory', 'coaching', 'privacy', 'notifications'])(
    'does not open the hidden #settings/%s pane for an admin',
    async (pane) => {
      auth.role = 'admin';
      window.history.replaceState(null, '', `/#settings/${pane}`);
      await act(async () => {
        renderDashboard();
      });
      await waitFor(() => expect(window.location.hash).toBe('#settings'));
    },
  );

  it('opens the section an admin\'s hash names', async () => {
    auth.role = 'admin';
    window.history.replaceState(null, '', '/#settings/account');
    await act(async () => {
      renderDashboard();
    });
    expect((await screen.findByTestId('user-settings')).textContent).toBe('account');
    expect(window.location.hash).toBe('#settings/account');
  });

  it('survives a malformed section escape on first load', async () => {
    auth.role = 'admin';
    window.history.replaceState(null, '', '/#settings/%E0%A4%A');
    await act(async () => {
      renderDashboard();
    });
    expect((await screen.findByTestId('user-settings')).textContent).toBe('default');
    await waitFor(() => expect(window.location.hash).toBe('#settings'));
  });

  it('rewrites a hidden pane typed while the menu is already open', async () => {
    auth.role = 'admin';
    window.history.replaceState(null, '', '/#settings');
    await act(async () => {
      renderDashboard();
    });
    await waitFor(() => expect(window.location.hash).toBe('#settings'));

    await act(async () => {
      window.location.hash = '#settings/memory';
    });

    await waitFor(() => expect(window.location.hash).toBe('#settings'));
  });

  it('keeps a hidden pane open for an athlete', async () => {
    auth.role = 'user';
    window.history.replaceState(null, '', '/#settings/memory');
    await act(async () => {
      renderDashboard();
    });
    expect(await screen.findByTestId('settings-shell')).toBeInTheDocument();
    expect(window.location.hash).toBe('#settings/memory');
  });

  it('rewrites #admin-tokens typed while the token screen is open, leaving no extra history entry', async () => {
    auth.role = 'admin';
    window.history.replaceState(null, '', '/#connections');
    await act(async () => {
      renderDashboard();
    });
    const entries = window.history.length;

    await act(async () => {
      window.location.hash = '#admin-tokens';
    });

    await waitFor(() => expect(window.location.hash).toBe('#connections'));
    // The typed hash itself is one entry; the rewrite replaces it rather than adding another.
    expect(window.history.length).toBe(entries + 1);
  });

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
