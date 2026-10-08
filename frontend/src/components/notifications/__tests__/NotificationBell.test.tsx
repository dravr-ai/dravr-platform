// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the header bell — the unread count on it and in its name, the notifications in a sheet, a row that leaves
// ABOUTME: The rail no longer carries notifications (carnet#820), so this bell is the one way to them on every surface

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { ThemeProvider } from '../../../hooks/useTheme';
import { NotificationBell } from '../NotificationBell';

const api = vi.hoisted(() => ({
  getUnreadCount: vi.fn<() => Promise<{ unread_count: number }>>(),
  listNotifications: vi.fn(),
  markAsRead: vi.fn(),
}));

vi.mock('../../../services/api', () => ({
  notificationsApi: {
    getUnreadCount: api.getUnreadCount,
    listNotifications: api.listNotifications,
    markAsRead: api.markAsRead,
  },
}));

const ROW = {
  id: 'n-1',
  category: 'coach',
  title: 'Message from your agent',
  body: 'Tempo sent you a message',
  data: { screen: 'activities' },
  read_at: null,
  created_at: '2026-10-07T10:00:00Z',
  collapsed_count: 1,
};

function renderBell(onNavigate = vi.fn()) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <ThemeProvider>
        <NotificationBell onNavigate={onNavigate} />
      </ThemeProvider>
    </QueryClientProvider>,
  );
  return { onNavigate };
}

beforeEach(() => {
  vi.clearAllMocks();
  api.listNotifications.mockResolvedValue({ data: [ROW], total: 1, unread_count: 1 });
  api.markAsRead.mockResolvedValue(undefined);
});

describe('NotificationBell', () => {
  it('carries the unread count on the bell and in its name', async () => {
    api.getUnreadCount.mockResolvedValue({ unread_count: 3 });
    renderBell();

    const bell = await screen.findByRole('button', { name: 'Notifications, 3 unread' });
    expect(within(bell).getByTestId('notification-bell-badge')).toHaveTextContent('3');
  });

  it('shows no badge when everything is read', async () => {
    api.getUnreadCount.mockResolvedValue({ unread_count: 0 });
    renderBell();

    await waitFor(() => expect(api.getUnreadCount).toHaveBeenCalled());
    expect(screen.getByRole('button', { name: 'Notifications' })).toBeInTheDocument();
    expect(screen.queryByTestId('notification-bell-badge')).toBeNull();
  });

  it('opens the notifications in a sheet, and a row that goes somewhere closes it there', async () => {
    api.getUnreadCount.mockResolvedValue({ unread_count: 1 });
    const { onNavigate } = renderBell();

    await userEvent.click(await screen.findByRole('button', { name: 'Notifications, 1 unread' }));
    const sheet = await screen.findByRole('dialog', { name: 'Notifications' });
    // The sheet's heading names it; the panel's own page header gives way.
    expect(within(sheet).getAllByRole('heading', { name: 'Notifications' })).toHaveLength(1);

    await userEvent.click(await within(sheet).findByTestId('notification-row-n-1'));
    expect(api.markAsRead).toHaveBeenCalledWith('n-1');
    expect(onNavigate).toHaveBeenCalledWith('home');
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  });
});
