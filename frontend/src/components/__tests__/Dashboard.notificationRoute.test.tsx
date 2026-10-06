// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Locks where a notification row takes the athlete — its thread, Home, or nowhere shown as a link
// ABOUTME: Regression for the fitness-improvement tap that landed on an empty chat saying nothing about the score

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, act, waitFor, within } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { NotificationItem } from '@pierre/shared-types';
import Dashboard from '../Dashboard';
import { ThemeProvider } from '../../hooks/useTheme';
import type { PendingComposerAction } from '../ChatTab';

const chatTabProps = vi.fn();
const markAsRead = vi.fn();

function notification(overrides: Partial<NotificationItem>): NotificationItem {
  return {
    id: 'notif-1',
    category: 'achievement',
    notification_type: 'fitness_improvement',
    title: 'Fitness improvement detected',
    body: 'Your Fitness Score rose to 48',
    data: null,
    image_url: null,
    read_at: new Date().toISOString(),
    delivered_at: null,
    opened_at: null,
    created_at: new Date().toISOString(),
    ...overrides,
  };
}

const FEED: NotificationItem[] = [
  // Fired from the agent's own tool call: it names the thread it answered in.
  notification({
    id: 'fitness-in-thread',
    data: { screen: 'coach', action: 'chat', id: 'conv-fitness-1' },
  }),
  // Stored under a retired screen, before the conversation travelled with it.
  notification({
    id: 'fitness-stored',
    title: 'Older fitness improvement',
    data: { screen: 'stats' },
    read_at: null,
  }),
  notification({
    id: 'personal-record',
    title: 'New personal record!',
    body: 'New 10 km record: 44:14',
    data: { screen: 'activity', id: 'act-1' },
  }),
  notification({
    id: 'agent-message',
    category: 'coach',
    notification_type: 'coach_message',
    title: 'Message from your agent',
    body: 'Marathon Agent sent you a message',
    data: { screen: 'coach', action: 'chat', id: 'conv-abc-123' },
    actions: [{ id: 'reply', title: 'Reply', action_type: 'quick_reply' }],
  }),
  // A sync failure stored when its screen was `settings`, which no longer
  // names a destination: its Reconnect button would lead nowhere.
  notification({
    id: 'sync-stored',
    category: 'system',
    notification_type: 'sync_failure',
    title: 'Sync failed',
    body: 'Strava could not sync',
    data: { screen: 'settings', action: 'reconnect' },
    actions: [{ id: 'reconnect', title: 'Reconnect', action_type: 'open_screen' }],
  }),
];

vi.mock('../dashboard/index', () => ({
  ConversationList: () => null,
  useUnreadConversationsCount: () => 0,
  usePendingUsersCount: () => 0,
  useStoreStatsPendingCount: () => 0,
}));

// The real panel renders against a fixed feed; its hooks are the network edge.
vi.mock('../../hooks/useNotifications', () => ({
  useUnreadCount: () => ({ unreadCount: 1, isLoading: false }),
  useNotificationFeed: () => ({ notifications: FEED, total: FEED.length, unreadCount: 1, isLoading: false }),
  useNotificationActions: () => ({
    markAsRead,
    markAllAsRead: vi.fn(),
    deleteNotification: vi.fn(),
    isMarkingAllRead: false,
  }),
}));

vi.mock('../ChatTab', () => ({
  default: (props: {
    layout?: string;
    selectedConversation: string | null;
    pendingComposerAction?: PendingComposerAction | null;
  }) => {
    // Home is ChatTab's personal layout; only the Groups tab's props are asserted here.
    if (props.layout === 'personal') return <div data-testid="home-tab" />;
    chatTabProps({ selected: props.selectedConversation, action: props.pendingComposerAction ?? null });
    return <div data-testid="chat-tab" />;
  },
}));

// Home's own rendering is covered by its tests; here it only has to be where
// a personal record lands.
vi.mock('../home/Home', () => ({ HomeBriefing: () => null }));
vi.mock('../home/TodayPeek', () => ({ TodayPeek: () => null }));
// Home opens on the latest personal thread; these specs assert nothing about
// which one, and no #chat link here names a thread the list knows.
vi.mock('../../hooks/useConversationList', () => ({
  useLatestPersonalConversation: () => ({ id: null, isLoading: false }),
  useConversationScope: () => null,
}));

vi.mock('../ConnectProviderBanner', () => ({ ConnectProviderBanner: () => null }));

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
      <ThemeProvider>
        <Dashboard />
      </ThemeProvider>
    </QueryClientProvider>,
  );
}

async function openNotifications() {
  await act(async () => {
    renderDashboard();
  });
  // The panel is lazy: wait for it outside `act`, which would hold its import.
  await screen.findByTestId('notification-row-fitness-in-thread');
}

describe('Dashboard — where a notification row goes', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.history.replaceState(null, '', '/#notifications');
  });

  it('opens the thread a fitness improvement was computed in, with nothing pre-typed', async () => {
    await openNotifications();
    const row = screen.getByTestId('notification-row-fitness-in-thread');
    expect(row).toHaveClass('cursor-pointer');

    await act(async () => {
      row.click();
    });

    expect(await screen.findByTestId('chat-tab')).toBeInTheDocument();
    await waitFor(() => expect(window.location.hash).toBe('#chat/conv-fitness-1'));
    expect(chatTabProps).toHaveBeenLastCalledWith({ selected: 'conv-fitness-1', action: null });
  });

  it('opens Home for a personal record', async () => {
    await openNotifications();

    await act(async () => {
      screen.getByTestId('notification-row-personal-record').click();
    });

    expect(await screen.findByTestId('home-tab')).toBeInTheDocument();
    await waitFor(() => expect(window.location.hash).toBe('#home'));
  });

  it('shows a row with nowhere to go as information: no link, no navigation, still marked read', async () => {
    await openNotifications();
    const row = screen.getByTestId('notification-row-fitness-stored');
    expect(row).not.toHaveClass('cursor-pointer');

    await act(async () => {
      row.click();
    });

    expect(markAsRead).toHaveBeenCalledExactlyOnceWith('fitness-stored');
    expect(window.location.hash).toBe('#notifications');
    expect(screen.queryByTestId('chat-tab')).not.toBeInTheDocument();
  });

  it('offers no action button that would lead nowhere', async () => {
    await openNotifications();
    const stored = screen.getByTestId('notification-row-sync-stored');
    expect(within(stored).queryByRole('button', { name: 'Reconnect' })).not.toBeInTheDocument();
    expect(stored).toHaveTextContent('Strava could not sync');
  });

  it("opens an agent message's thread from its Reply button", async () => {
    await openNotifications();
    const reply = within(screen.getByTestId('notification-row-agent-message')).getByRole('button', {
      name: 'Reply',
    });

    await act(async () => {
      reply.click();
    });

    expect(await screen.findByTestId('chat-tab')).toBeInTheDocument();
    await waitFor(() => expect(window.location.hash).toBe('#chat/conv-abc-123'));
    expect(chatTabProps).toHaveBeenLastCalledWith({ selected: 'conv-abc-123', action: null });
  });
});
