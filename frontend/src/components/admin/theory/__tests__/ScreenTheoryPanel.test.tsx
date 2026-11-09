// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The admin shell opens each operator screen with its own theory panel, and an athlete's screen with none
// ABOUTME: Mounts the Dashboard at several tabs with the screens stubbed and reads the panel's question, since and commits

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, act, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import Dashboard from '../../../Dashboard';
import { SCREEN_THEORY } from '../screenTheory';

vi.mock('../../../ClaimVerdictsTab', () => ({
  default: () => <div data-testid="claim-verdicts-tab" />,
}));
vi.mock('../../../UserManagement', () => ({
  default: () => <div data-testid="users-tab" />,
}));
vi.mock('../../../AuditTrail', () => ({
  default: () => <div data-testid="audit-tab" />,
}));
vi.mock('../../../UserSettings', () => ({
  default: () => <div data-testid="settings-tab" />,
}));
vi.mock('../../../dashboard/index', () => ({
  ConversationList: () => null,
  useUnreadConversationsCount: () => 0,
  usePendingUsersCount: () => 0,
  useStoreStatsPendingCount: () => 0,
}));
vi.mock('../../../../hooks/useNotifications', () => ({
  useUnreadCount: () => ({ unreadCount: 0, isLoading: false }),
}));
vi.mock('../../../home/PersonalChat', () => ({
  default: () => <div data-testid="home-tab" />,
}));
vi.mock('../../../home/Home', () => ({ HomeBriefing: () => null }));
vi.mock('../../../home/TodayPeek', () => ({ TodayPeek: () => null }));
vi.mock('../../../../hooks/useConversationList', () => ({
  usePersonalConversation: () => ({ id: null, isLoading: false }),
  useConversationScope: () => null,
}));
vi.mock('../../../ConnectProviderBanner', () => ({
  ConnectProviderBanner: () => null,
}));

const viewer = vi.hoisted(() => ({ role: 'admin' }));
vi.mock('../../../../hooks/useAuth', () => ({
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
vi.mock('../../../../services/api', () => ({ featureFlagsApi: {} }));
vi.mock('../../../../services/analytics', () => ({ track: vi.fn() }));

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

describe('admin screen theory panel', () => {
  beforeEach(() => {
    viewer.role = 'admin';
  });

  it.each([
    { tab: 'users', screenId: 'users-tab' },
    { tab: 'claim-verdicts', screenId: 'claim-verdicts-tab' },
    { tab: 'audit', screenId: 'audit-tab' },
    { tab: 'settings', screenId: 'settings-tab' },
  ])('opens $tab with its own question, since and commits', async ({ tab, screenId }) => {
    await openAt(`#${tab}`);
    await screen.findByTestId(screenId);
    const entry = SCREEN_THEORY[tab];

    const panels = screen.getAllByTestId('screen-theory');
    expect(panels).toHaveLength(1);
    const panel = panels[0] as HTMLDetailsElement;
    expect(panel.dataset.tab).toBe(tab);
    expect(panel.open).toBe(false);
    // Closed, the body is not mounted: no hidden copy for a text search to hit.
    expect(within(panel).queryByText(entry.theory)).toBeNull();

    await userEvent.click(within(panel).getByText('How this screen works'));
    expect(panel.open).toBe(true);

    expect(within(panel).getByText(entry.question, { exact: false })).toBeInTheDocument();
    expect(within(panel).getByText(entry.theory)).toBeInTheDocument();
    expect(within(panel).getByTestId('screen-theory-since')).toHaveTextContent(entry.since);
    for (const line of entry.computation) {
      expect(within(panel).getByText(line)).toBeInTheDocument();
    }
    for (const commit of entry.commits) {
      expect(within(panel).getByText(commit.sha)).toBeInTheDocument();
    }
  });

  it('shows the Audit panel its own origin, not another screen\'s', async () => {
    await openAt('#audit');
    await screen.findByTestId('audit-tab');

    const panel = screen.getByTestId('screen-theory');
    await userEvent.click(within(panel).getByText('How this screen works'));
    expect(within(panel).getByTestId('screen-theory-since')).toHaveTextContent('2026-11-04');
    expect(within(panel).getByText('19c01f6c0')).toBeInTheDocument();
    expect(within(panel).queryByText(SCREEN_THEORY.users.question, { exact: false })).toBeNull();
  });

  it('draws no panel on an athlete\'s screen', async () => {
    viewer.role = 'user';
    await openAt('#home');
    await screen.findByTestId('home-tab');

    expect(screen.queryByTestId('screen-theory')).toBeNull();
  });
});
