// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Locks the notification centre's way into chat — a threadless notification opens a fresh thread quoting it
// ABOUTME: Regression for the fitness-improvement tap that landed on a bare chat saying nothing about the event

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, act, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { i18n } from '@pierre/i18n';
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
    data: { screen: 'stats' },
    image_url: null,
    read_at: new Date().toISOString(),
    delivered_at: null,
    opened_at: null,
    created_at: new Date().toISOString(),
    ...overrides,
  };
}

const FEED: NotificationItem[] = [
  notification({ id: 'fitness-1' }),
  notification({
    id: 'agent-1',
    category: 'coach',
    notification_type: 'agent_message',
    title: 'Message from your agent',
    body: 'Marathon Agent sent you a message',
    data: { screen: 'coach', action: 'chat', id: 'conv-abc-123' },
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
  useUnreadCount: () => ({ unreadCount: 0, isLoading: false }),
  useNotificationFeed: () => ({ notifications: FEED, total: FEED.length, unreadCount: 0, isLoading: false }),
  useNotificationActions: () => ({
    markAsRead,
    markAllAsRead: vi.fn(),
    deleteNotification: vi.fn(),
    isMarkingAllRead: false,
  }),
}));

vi.mock('../ChatTab', () => ({
  default: (props: {
    selectedConversation: string | null;
    pendingComposerAction?: PendingComposerAction | null;
  }) => {
    chatTabProps({ selected: props.selectedConversation, action: props.pendingComposerAction ?? null });
    return <div data-testid="chat-tab" />;
  },
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

describe('Dashboard — a notification opens chat', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.history.replaceState(null, '', '/#notifications');
  });

  it('opens a fitness improvement in a fresh thread whose composer quotes it', async () => {
    await act(async () => {
      renderDashboard();
    });
    const row = await screen.findByText('Fitness improvement detected');
    await act(async () => {
      row.click();
    });

    expect(await screen.findByTestId('chat-tab')).toBeInTheDocument();
    await waitFor(() => expect(window.location.hash).toBe('#chat'));
    const draft = i18n.t('notifications.askDraft', {
      title: 'Fitness improvement detected',
      body: 'Your Fitness Score rose to 48',
    });
    expect(draft).toContain('Your Fitness Score rose to 48');
    expect(chatTabProps).toHaveBeenLastCalledWith({ selected: null, action: { kind: 'draft', text: draft } });
  });

  it('still opens an agent message on its own thread, with no draft', async () => {
    await act(async () => {
      renderDashboard();
    });
    const row = await screen.findByText('Message from your agent');
    await act(async () => {
      row.click();
    });

    expect(await screen.findByTestId('chat-tab')).toBeInTheDocument();
    await waitFor(() => expect(window.location.hash).toBe('#chat/conv-abc-123'));
    expect(chatTabProps).toHaveBeenLastCalledWith({ selected: 'conv-abc-123', action: null });
  });
});
