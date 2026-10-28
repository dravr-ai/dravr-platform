// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the notification centre's chrome in the athlete's language — the count line, the category word, the collapsed count
// ABOUTME: Each read « 1 unread · 1 total », « Agent » and « +1 similar » under French chrome (carnet#1191)

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, within } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { i18n } from '@pierre/i18n';
import { ThemeProvider } from '../../../hooks/useTheme';
import NotificationsPanel from '../NotificationsPanel';

const api = vi.hoisted(() => ({
  getUnreadCount: vi.fn(),
  listNotifications: vi.fn(),
  markAsRead: vi.fn(),
  markAllAsRead: vi.fn(),
  deleteNotification: vi.fn(),
}));

vi.mock('../../../services/api', () => ({
  notificationsApi: api,
}));

/** A coach's plan notice: one person's act, in category `coach`. */
const PLAN_PUBLISHED = {
  id: 'n-plan',
  category: 'coach',
  title: 'Plan publié par Camille',
  body: 'Ton plan pour le Marathon de Montréal est en ligne.',
  data: { screen: 'activities' },
  read_at: null,
  created_at: new Date().toISOString(),
  collapsed_count: 2,
};

const READ_ROW = { ...PLAN_PUBLISHED, id: 'n-read', read_at: new Date().toISOString(), collapsed_count: 1 };

function renderPanel(embedded = false) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <ThemeProvider>
        <NotificationsPanel embedded={embedded} />
      </ThemeProvider>
    </QueryClientProvider>,
  );
}

beforeEach(async () => {
  vi.clearAllMocks();
  await i18n.changeLanguage('fr');
});

afterEach(async () => {
  await i18n.changeLanguage('en');
});

describe('NotificationsPanel in French', () => {
  it('counts one unread notification in French, never in English', async () => {
    api.listNotifications.mockResolvedValue({ data: [PLAN_PUBLISHED], total: 1, unread_count: 1 });
    renderPanel(true);

    const summary = await screen.findByText('1 non lue · 1 au total');
    expect(summary).toHaveAttribute('data-testid', 'notifications-summary');
    expect(summary.textContent).not.toMatch(/unread|\d total/);
  });

  it('agrees the count with many unread', async () => {
    api.listNotifications.mockResolvedValue({ data: [PLAN_PUBLISHED], total: 5, unread_count: 3 });
    renderPanel(true);

    expect(await screen.findByText('3 non lues · 5 au total')).toBeInTheDocument();
  });

  it('says caught up when nothing is unread', async () => {
    api.listNotifications.mockResolvedValue({ data: [READ_ROW], total: 1, unread_count: 0 });
    renderPanel(true);

    expect(await screen.findByText('Tu es à jour · 1 au total')).toBeInTheDocument();
  });

  it('files a coach\'s plan notice under Coaching, not under the agent', async () => {
    api.listNotifications.mockResolvedValue({ data: [PLAN_PUBLISHED], total: 1, unread_count: 1 });
    renderPanel();

    const row = await screen.findByTestId('notification-row-n-plan');
    expect(within(row).getByText('Coaching')).toBeInTheDocument();
    expect(within(row).queryByText('Agent')).toBeNull();
    // The category tab reads the same word.
    expect(screen.getByRole('button', { name: 'Coaching' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Agent' })).toBeNull();
  });

  it('counts the notifications a row collapsed in French', async () => {
    api.listNotifications.mockResolvedValue({ data: [PLAN_PUBLISHED], total: 1, unread_count: 1 });
    renderPanel();

    const row = await screen.findByTestId('notification-row-n-plan');
    expect(within(row).getByText('+1 similaire')).toBeInTheDocument();
    expect(within(row).queryByText(/similar\b/)).toBeNull();
  });
});
