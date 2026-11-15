// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Locks where Home routes a coach still waiting for coach access (carnet#1529): the pending coach Home, never the athlete's
// ABOUTME: A coach who trains keeps their own Home behind the switch; one who does not has none, with access or without; a grant turns Home into the coach Home

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, act, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import Dashboard from '../Dashboard';

const session = vi.hoisted(() => ({
  user: { id: 'u-1', email: 'marc@acme.com', display_name: 'Marc', role: 'user', manages_roster: false },
}));
const onboarding = vi.hoisted(() => ({
  status: { needs_provider_connection: true, steps: [] as { step_id: string; status: string }[], coaches_others: true },
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
vi.mock('../home/PersonalChat', () => ({
  default: ({ today }: { today?: unknown }) => (
    <div data-testid="home-tab" data-today={today === undefined ? 'none' : 'shown'}>
      Home surface
    </div>
  ),
}));
vi.mock('../coach-home/PendingCoachHome', () => ({
  default: ({ trains }: { trains: boolean }) => (
    <div data-testid="pending-coach-home" data-trains={String(trains)}>
      Pending coach Home
    </div>
  ),
}));
vi.mock('../coach-home/CoachHome', () => ({
  default: ({ trains, route }: { trains?: boolean; route: { view: string } }) => (
    <div data-testid="coach-home" data-trains={String(trains)} data-view={route.view}>
      Coach Home
    </div>
  ),
}));
vi.mock('../ChatTab', () => ({ default: () => null }));
vi.mock('../home/Home', () => ({ HomeBriefing: () => null }));
vi.mock('../home/TodayPeek', () => ({ TodayPeek: () => null }));
vi.mock('../../hooks/useConversationList', () => ({
  usePersonalConversation: () => ({ id: null, isLoading: false }),
  useConversationScope: () => null,
}));
vi.mock('../ConnectProviderBanner', () => ({ ConnectProviderBanner: () => null }));
vi.mock('../../hooks/useAuth', () => ({
  useAuth: () => ({ user: session.user, logout: vi.fn(), updateUser: vi.fn(), isAuthenticated: true, isLoading: false }),
}));
vi.mock('../../services/api', () => ({
  featureFlagsApi: {},
  userApi: {
    getOnboardingStatus: () => Promise.resolve(onboarding.status),
    getCoachAccessRequest: () => Promise.resolve({ request: null }),
  },
}));
vi.mock('../../services/analytics', () => ({ track: vi.fn() }));

const WAIVED = [
  { step_id: 'about_you', status: 'not_applicable' },
  { step_id: 'parq', status: 'not_applicable' },
];

function renderDashboard() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  const tree = () => (
    <QueryClientProvider client={queryClient}>
      <Dashboard />
    </QueryClientProvider>
  );
  const view = render(tree());
  return { ...view, rerenderSame: () => view.rerender(tree()) };
}

describe('Dashboard — Home while coach access is pending', () => {
  beforeEach(() => {
    window.history.replaceState(null, '', '/');
    session.user = { ...session.user, manages_roster: false };
    onboarding.status = { needs_provider_connection: true, steps: WAIVED, coaches_others: true };
  });

  it('lands a coach who does not train on the pending coach Home, with no switch', async () => {
    await act(async () => {
      renderDashboard();
    });
    const home = await screen.findByTestId('pending-coach-home');
    expect(home).toHaveAttribute('data-trains', 'false');
    expect(screen.queryByTestId('home-tab')).toBeNull();
  });

  it('keeps every Home view of a coach who does not train on the pending Home, even #home/me', async () => {
    window.history.replaceState(null, '', '/#home/me');
    await act(async () => {
      renderDashboard();
    });
    expect(await screen.findByTestId('pending-coach-home')).toBeInTheDocument();
    expect(screen.queryByTestId('home-tab')).toBeNull();
  });

  it('opens a coach who does not train on their own thread without Today, by a link', async () => {
    window.history.replaceState(null, '', '/#home/chat/conv-1');
    await act(async () => {
      renderDashboard();
    });
    expect(await screen.findByTestId('home-tab')).toHaveAttribute('data-today', 'none');
  });

  it('gives a pending coach who trains the switch, and their own Home behind it', async () => {
    onboarding.status = { ...onboarding.status, steps: [] };
    window.history.replaceState(null, '', '/#home/me');
    await act(async () => {
      renderDashboard();
    });
    expect(await screen.findByTestId('coach-home-personal')).toBeInTheDocument();
    expect(screen.getByTestId('home-tab')).toHaveAttribute('data-today', 'shown');
    expect(window.location.hash).toBe('#home/me');
  });

  it('turns Home into the coach Home once the session holds coach access', async () => {
    const view = renderDashboard();
    expect(await screen.findByTestId('pending-coach-home')).toBeInTheDocument();

    // The grant refresh patches the signed-in user; no reload.
    session.user = { ...session.user, manages_roster: true };
    await act(async () => {
      view.rerenderSame();
    });
    expect(await screen.findByTestId('coach-home')).toBeInTheDocument();
    expect(screen.queryByTestId('pending-coach-home')).toBeNull();
  });

  it('leaves an athlete on the athlete Home', async () => {
    onboarding.status = { needs_provider_connection: true, steps: [], coaches_others: false };
    await act(async () => {
      renderDashboard();
    });
    expect(await screen.findByTestId('home-tab')).toHaveAttribute('data-today', 'shown');
    expect(screen.queryByTestId('pending-coach-home')).toBeNull();
  });
});

describe('Dashboard — Home of a coach with access', () => {
  beforeEach(() => {
    window.history.replaceState(null, '', '/');
    session.user = { ...session.user, manages_roster: true };
  });

  it('gives a coach who does not train no « Mon entraînement »: #home/me is their coach Home', async () => {
    onboarding.status = { needs_provider_connection: true, steps: WAIVED, coaches_others: true };
    window.history.replaceState(null, '', '/#home/me');
    await act(async () => {
      renderDashboard();
    });
    const home = await screen.findByTestId('coach-home');
    expect(home).toHaveAttribute('data-trains', 'false');
    expect(home).toHaveAttribute('data-view', 'queue');
    expect(screen.queryByTestId('coach-home-personal')).toBeNull();
    expect(screen.queryByTestId('home-tab')).toBeNull();
  });

  it('opens their own thread without Today, from the conversation list', async () => {
    onboarding.status = { needs_provider_connection: true, steps: WAIVED, coaches_others: true };
    window.history.replaceState(null, '', '/#home/chat/conv-1');
    await act(async () => {
      renderDashboard();
    });
    // Once the onboarding answer reads, Today leaves the thread.
    await waitFor(() => expect(screen.getByTestId('home-tab')).toHaveAttribute('data-today', 'none'));
  });

  it('keeps the switch and their own Home for a coach who trains', async () => {
    onboarding.status = { needs_provider_connection: false, steps: [], coaches_others: true };
    await act(async () => {
      renderDashboard();
    });
    expect(await screen.findByTestId('coach-home')).toHaveAttribute('data-trains', 'true');

    window.history.replaceState(null, '', '/#home/me');
    await act(async () => {
      renderDashboard();
    });
    expect(await screen.findByTestId('coach-home-personal')).toBeInTheDocument();
    expect(screen.getByTestId('home-tab')).toHaveAttribute('data-today', 'shown');
  });
});
