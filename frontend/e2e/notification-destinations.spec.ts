// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Drives the notification centre in a real browser — each row opens its thread, Home, or nothing
// ABOUTME: A row with nowhere to go shows no pointer and no action button, so no tap leads to an empty chat

import { test, expect, type Page } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard } from './test-helpers';

const NOW = new Date().toISOString();

function item(overrides: Record<string, unknown>) {
  return {
    id: 'notif-1',
    category: 'achievement',
    notification_type: 'fitness_improvement',
    title: 'Fitness improvement detected',
    body: 'Your Fitness Score rose to 48',
    data: null,
    image_url: null,
    read_at: NOW,
    delivered_at: null,
    opened_at: null,
    created_at: NOW,
    ...overrides,
  };
}

const FEED = [
  // Fired from the agent's own tool call: it names the thread it answered in.
  item({ id: 'fitness-in-thread', data: { screen: 'coach', action: 'chat', id: 'conv-fitness-1' } }),
  // Stored under a retired screen, before the conversation travelled with it.
  item({ id: 'fitness-stored', title: 'Older fitness improvement', data: { screen: 'stats' } }),
  item({
    id: 'personal-record',
    title: 'New personal record!',
    body: 'New 10 km record: 44:14',
    data: { screen: 'activity', id: 'act-1' },
  }),
  // A sync failure stored when its screen was `settings`, which names nothing now.
  item({
    id: 'sync-stored',
    category: 'system',
    notification_type: 'sync_failure',
    title: 'Sync failed',
    body: 'Strava could not sync',
    data: { screen: 'settings', action: 'reconnect' },
    actions: [{ id: 'reconnect', title: 'Reconnect', action_type: 'open_screen' }],
  }),
];

async function openNotificationCentre(page: Page) {
  await setupDashboardMocks(page, { role: 'user' });
  // Registered after the shared mocks, so this feed wins for the list query.
  await page.route('**/api/notifications**', async (route) => {
    const url = route.request().url();
    const body = url.includes('unread-count')
      ? { count: 0 }
      : { data: FEED, total: FEED.length, unread_count: 0 };
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });
  });
  await loginToDashboard(page);
  await page.goto('/#notifications');
  await expect(page.getByTestId('notification-row-fitness-in-thread')).toBeVisible({ timeout: 15000 });
}

async function cursorOf(page: Page, testId: string): Promise<string> {
  return page.getByTestId(testId).evaluate((row) => getComputedStyle(row).cursor);
}

test.describe('Notification destinations', () => {
  test('a fitness improvement opens the thread the agent computed it in', async ({ page }) => {
    await openNotificationCentre(page);
    expect(await cursorOf(page, 'notification-row-fitness-in-thread')).toBe('pointer');

    await page.getByTestId('notification-row-fitness-in-thread').click();

    await expect(page).toHaveURL(/#chat\/conv-fitness-1$/);
  });

  test('a personal record opens Home, where the latest activities are', async ({ page }) => {
    await openNotificationCentre(page);

    await page.getByTestId('notification-row-personal-record').click();

    await expect(page).toHaveURL(/#home$/);
    await expect(page.getByTestId('home-page')).toBeVisible();
  });

  test('a row with nowhere to go is information, not a link', async ({ page }) => {
    await openNotificationCentre(page);
    expect(await cursorOf(page, 'notification-row-fitness-stored')).not.toBe('pointer');

    await page.getByTestId('notification-row-fitness-stored').click();

    // Still on the centre: the empty chat it used to land on is the dead end.
    await expect(page).toHaveURL(/#notifications$/);
    await expect(page.getByTestId('notification-row-fitness-stored')).toBeVisible();
  });

  test('a stored sync failure offers no Reconnect button that would lead nowhere', async ({ page }) => {
    await openNotificationCentre(page);
    const row = page.getByTestId('notification-row-sync-stored');

    await expect(row).toContainText('Strava could not sync');
    await expect(row.getByRole('button', { name: 'Reconnect' })).toHaveCount(0);
  });
});
