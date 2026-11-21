// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Locks the admin-only tabs behind the role that the server serves them to
// ABOUTME: Every admin tab's hash lands a user on Home, on first load, typed later, and when the role is lost under an open pane; an admin opens it

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, act, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import Dashboard from '../Dashboard';
import { ADMIN_ONLY_TABS } from '../../constants/adminTabs';

const auth = vi.hoisted(() => ({ role: 'admin' }));
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
    user: { id: 'op-1', email: 'op@acme.com', display_name: 'Admin', role: auth.role },
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

describe('Dashboard admin tabs', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    drawn.as = [];
    window.history.replaceState(null, '', '/');
  });

  // The admin panes carry no branch for a viewer without the role: this guard
  // is what keeps one from ever mounting, so it is pinned for every admin tab.
  it.each([...ADMIN_ONLY_TABS])(
    'resolves a user #%s deep link to Home on first load',
    async (tab) => {
      auth.role = 'user';
      window.history.replaceState(null, '', `/#${tab}`);

      await act(async () => {
        renderDashboard();
      });

      expect(await screen.findByTestId('home-tab')).toBeInTheDocument();
      expect(screen.queryByTestId('users-tab')).toBeNull();
      expect(screen.queryByTestId('impersonation-log-tab')).toBeNull();
      expect(window.location.hash).toBe('#home');
    },
  );

  it('offers the settings tabs only to the role the server serves them to', async () => {
    auth.role = 'user';
    const userView = await act(async () => renderDashboard());
    expect(await screen.findByTestId('home-tab')).toBeInTheDocument();
    expect(screen.queryByText('Harness Config')).toBeNull();
    expect(screen.queryByText('Impersonation log')).toBeNull();
    userView.unmount();

    auth.role = 'admin';
    await act(async () => {
      renderDashboard();
    });
    await waitFor(() =>
      expect(screen.queryByText('Harness Config')).not.toBeNull(),
    );
    expect(screen.queryByText('Impersonation log')).not.toBeNull();
  });

  it('opens an admin #admin-tokens bookmark on the one token screen', async () => {
    auth.role = 'admin';
    window.history.replaceState(null, '', '/#admin-tokens');
    await act(async () => {
      renderDashboard();
    });
    await waitFor(() => expect(window.location.hash).toBe('#connections'));
  });

  it('offers an admin one token entry and no inbox in the rail', async () => {
    auth.role = 'admin';
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

  it('resolves every admin tab a user types after load to Home', async () => {
    auth.role = 'user';
    await act(async () => {
      renderDashboard();
    });
    expect(await screen.findByTestId('home-tab')).toBeInTheDocument();

    for (const tab of ADMIN_ONLY_TABS) {
      await act(async () => {
        window.location.hash = `#${tab}`;
      });

      await waitFor(() => expect(window.location.hash, `#${tab}`).toBe('#home'));
      expect(screen.getByTestId('home-tab'), `#${tab}`).toBeInTheDocument();
      expect(screen.queryByTestId('users-tab'), `#${tab}`).toBeNull();
      expect(screen.queryByTestId('impersonation-log-tab'), `#${tab}`).toBeNull();
    }
  });

  // The cached account renders first and the session's replaces it
  // (AuthContext), so an admin demoted since this browser last saw it mounts
  // as an admin and loses the role a render later, with no navigation.
  it.each([...ADMIN_ONLY_TABS])(
    'sends an account that loses the admin role on #%s to Home',
    async (tab) => {
      auth.role = 'admin';
      window.history.replaceState(null, '', `/#${tab}`);
      const queryClient = new QueryClient({
        defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
      });
      const tree = () => (
        <QueryClientProvider client={queryClient}>
          <Dashboard />
        </QueryClientProvider>
      );
      const view = await act(async () => render(tree()));
      // A tab this build gates off or moved opens the admin's own screen for
      // it; either way the admin is on an admin pane, not on Home.
      await waitFor(() => expect(window.location.hash).not.toBe(''));
      expect(ADMIN_ONLY_TABS.has(window.location.hash.replace(/^#/, ''))).toBe(true);
      expect(screen.queryByTestId('home-tab')).toBeNull();
      const entries = window.history.length;

      auth.role = 'user';
      await act(async () => {
        view.rerender(tree());
      });

      expect(await screen.findByTestId('home-tab')).toBeInTheDocument();
      expect(screen.queryByTestId('users-tab')).toBeNull();
      expect(screen.queryByTestId('impersonation-log-tab')).toBeNull();
      await waitFor(() => expect(window.location.hash).toBe('#home'));
      // The out-of-role entry is replaced, so Back does not return to it.
      expect(window.history.length).toBe(entries);
    },
  );

  // The effect that sends the account away runs after the render that lost
  // the role, so that render is the one a pane could still be drawn in. The
  // panes ask nothing about who is looking (the token screen's own non-admin
  // arms are gone): the Dashboard is what never draws one for a plain user.
  it.each([
    ['users', 'users-tab'],
    ['connections', 'connections-tab'],
    ['impersonation-log', 'impersonation-log-tab'],
  ])('never draws the #%s pane for the account that lost the admin role on it', async (tab, pane) => {
    auth.role = 'admin';
    window.history.replaceState(null, '', `/#${tab}`);
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
    });
    const tree = () => (
      <QueryClientProvider client={queryClient}>
        <Dashboard />
      </QueryClientProvider>
    );
    const view = await act(async () => render(tree()));
    // The control: the cached admin is drawn the pane.
    expect(await screen.findByTestId(pane)).toBeInTheDocument();
    expect(drawn.as).toContain('admin');

    auth.role = 'user';
    await act(async () => {
      view.rerender(tree());
    });

    expect(await screen.findByTestId('home-tab')).toBeInTheDocument();
    expect(screen.queryByTestId(pane)).toBeNull();
    expect(drawn.as).not.toContain('user');
  });

  it('draws each rail section header once for an admin', async () => {
    auth.role = 'admin';
    await act(async () => {
      renderDashboard();
    });
    expect(await screen.findByTestId('users-tab')).toBeInTheDocument();
    // The tab list appends Platform and Configuration entries after Developer.
    for (const section of ['Platform', 'Coaching', 'Configuration', 'Developer']) {
      expect(screen.getAllByText(section, { exact: true }), `${section} header`).toHaveLength(1);
    }
  });

  it('opens the impersonation log for an admin', async () => {
    auth.role = 'admin';
    window.history.replaceState(null, '', '/#impersonation-log');

    await act(async () => {
      renderDashboard();
    });

    expect(await screen.findByTestId('impersonation-log-tab')).toBeInTheDocument();
    expect(window.location.hash).toBe('#impersonation-log');
  });
});
