// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Locks where a bookmark of a retired or unknown tab lands, for an admin and an athlete
// ABOUTME: #eval-harness opens Claim Verdicts; anything the Dashboard does not draw opens the role's home and rewrites the hash

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, act } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import Dashboard from '../Dashboard';

vi.mock('../ClaimVerdictsTab', () => ({
  default: () => <div data-testid="claim-verdicts-tab" />,
}));
vi.mock('../UserManagement', () => ({
  default: () => <div data-testid="users-tab" />,
}));
vi.mock('../dashboard/index', () => ({
  ConversationList: () => null,
  useUnreadConversationsCount: () => 0,
  usePendingUsersCount: () => 0,
  useStoreStatsPendingCount: () => 0,
}));

vi.mock('../../hooks/useNotifications', () => ({
  useUnreadCount: () => ({ unreadCount: 0, isLoading: false }),
}));

// Home is PersonalChat; the Groups tab is ChatTab.
vi.mock('../home/PersonalChat', () => ({
  default: () => <div data-testid="home-tab">Home surface</div>,
}));
vi.mock('../ChatTab', () => ({
  default: () => <div data-testid="chat-tab">Groups surface</div>,
}));

vi.mock('../StoreScreen', () => ({
  default: () => <div data-testid="discover-tab">Discover surface</div>,
}));

vi.mock('../home/Home', () => ({ HomeBriefing: () => null }));
vi.mock('../home/TodayPeek', () => ({ TodayPeek: () => null }));
// Home opens on the latest personal thread; these specs assert nothing about
// which one, and no #chat link here names a thread the list knows.
vi.mock('../../hooks/useConversationList', () => ({
  usePersonalConversation: () => ({ id: null, isLoading: false }),
  useConversationScope: () => null,
}));

vi.mock('../ConnectProviderBanner', () => ({
  ConnectProviderBanner: () => null,
}));

const viewer = vi.hoisted(() => ({ role: 'admin' }));
vi.mock('../../hooks/useAuth', () => ({
  useAuth: () => ({
    user: {
      id: 'u-1',
      email: 'someone@acme.com',
      display_name: 'Someone',
      role: viewer.role,
      is_admin: viewer.role !== 'user',
    },
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

async function openAt(hash: string) {
  window.history.replaceState(null, '', `/${hash}`);
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  await act(async () => {
    render(
      <QueryClientProvider client={client}>
        <Dashboard />
      </QueryClientProvider>,
    );
  });
}

describe('retired admin tabs', () => {
  beforeEach(() => {
    viewer.role = 'admin';
  });

  it('opens Claim Verdicts from a bookmarked #eval-harness', async () => {
    await openAt('#eval-harness');

    expect(await screen.findByTestId('claim-verdicts-tab')).toBeInTheDocument();
    expect(window.location.hash).toBe('#claim-verdicts');
  });

  it.each(['#agent-followups', '#agent-notes-audit'])(
    'opens the admin default from a bookmarked %s',
    async (hash) => {
      await openAt(hash);

      expect(await screen.findByTestId('users-tab')).toBeInTheDocument();
      expect(window.location.hash).toBe('#users');
    },
  );

  it('offers none of the retired tabs in the rail', async () => {
    await openAt('#users');
    await screen.findByTestId('users-tab');

    for (const name of ['shell.navEvalHarness', 'shell.navAgentFollowups', 'shell.navAgentNotesAudit']) {
      expect(screen.queryByRole('button', { name })).toBeNull();
    }
    for (const name of ['Eval Harness', 'Agent Followups', 'Agent Notes Audit']) {
      expect(screen.queryByRole('button', { name })).toBeNull();
    }
  });

  describe.each([
    { role: 'admin', hash: '#users', home: 'users-tab' },
    { role: 'user', hash: '#home', home: 'home-tab' },
  ])('an unknown tab for a $role', ({ role, hash, home }) => {
    beforeEach(() => {
      viewer.role = role;
    });

    it('opens home and rewrites the hash on a bookmark', async () => {
      await openAt('#no-such-tab');

      expect(await screen.findByTestId(home)).toBeInTheDocument();
      expect(window.location.hash).toBe(hash);
    });

    it('takes a sub-route with it', async () => {
      await openAt('#no-such-tab/some-id');

      expect(await screen.findByTestId(home)).toBeInTheDocument();
      expect(window.location.hash).toBe(hash);
    });

    it('opens home and rewrites the hash when typed into a running session', async () => {
      await openAt(hash);
      await screen.findByTestId(home);

      await act(async () => {
        window.location.hash = '#no-such-tab';
        window.dispatchEvent(new HashChangeEvent('hashchange'));
      });

      expect(await screen.findByTestId(home)).toBeInTheDocument();
      expect(window.location.hash).toBe(hash);
    });
  });
});
