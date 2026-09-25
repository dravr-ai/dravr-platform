// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Mobile-viewport E2E for the athlete Home — lands there, Home leads the bottom bar, the page fits the phone
// ABOUTME: Tapping an activity opens chat with the draft at this width too, where there is no rail and no logo

import { test, expect, type Page } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard } from './test-helpers';

const TODAY = '2026-09-24';

const PLAN = {
  goal_race: { name: 'Montreal Marathon', date: '2026-11-22', discipline: 'run_marathon', priority: 'A' },
  phases: [
    { kind: 'build', start: '2026-09-14', end: '2026-10-26', weeks: 6, purpose: 'raise the ceiling', intent: 'two hard days', current: true },
  ],
  weeks: [
    {
      week_start: '2026-09-21',
      focus: 'threshold volume',
      current: true,
      days: [
        { date: '2026-09-24', sport: 'run', workout: 'Tempo run', duration_min: 50, intensity: 'threshold', rest: false },
        { date: '2026-09-25', sport: 'rest', workout: '', intensity: '', rest: true },
        { date: '2026-09-27', sport: 'ride', workout: 'Long ride', duration_min: 180, intensity: 'Z2', rest: false },
      ],
    },
  ],
  weeks_deferred: 0,
};

const ACTIVITIES = [
  {
    id: 'act-2',
    provider: 'strava',
    name: 'Trainer spin',
    sport_type: 'virtual_ride',
    start_date: '2026-09-22T22:00:00Z',
    duration_seconds: 3600,
    distance_meters: 30000,
    elevation_gain_meters: null,
    has_gps: false,
    summary_polyline: null,
  },
  {
    id: 'act-1',
    provider: 'strava',
    name: 'Morning run',
    sport_type: 'run',
    start_date: '2026-09-18T11:00:00Z',
    duration_seconds: 3000,
    distance_meters: 10200,
    elevation_gain_meters: 85,
    has_gps: true,
    summary_polyline: '_p~iF~ps|U_ulLnnqC_mqNvxq`@',
  },
];

const CONVERSATION = {
  id: 'conv-home-mobile',
  title: 'New conversation',
  agent_id: null,
  created_at: '2026-09-24T10:00:00Z',
  updated_at: '2026-09-24T10:00:00Z',
  message_count: 0,
  unread_count: 0,
  last_message: null,
};

async function mockHome(page: Page) {
  await page.route('**/api/providers', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ providers: [{ provider: 'strava', connected: true, status: 'connected' }] }),
    });
  });
  await page.route('**/api/me/training-plan**', async (route) => {
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ plan: PLAN, today: TODAY }) });
  });
  await page.route('**/api/me/activities/recent**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ activities: ACTIVITIES, as_of: '2026-09-24T08:15:00Z', stale: false }),
    });
  });
  await page.route(`**/api/chat/conversations/${CONVERSATION.id}/messages**`, async (route) => {
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ messages: [] }) });
  });
  await page.route(/\/api\/chat\/conversations(\?.*)?$/, async (route, request) => {
    if (request.method() === 'POST') {
      await route.fulfill({ status: 201, contentType: 'application/json', body: JSON.stringify(CONVERSATION) });
      return;
    }
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ conversations: [], total: 0, limit: 50, offset: 0 }),
    });
  });
}

test.describe('Athlete Home — mobile viewport', () => {
  test.beforeEach(async ({ page }) => {
    await setupDashboardMocks(page, { role: 'user', email: 'alice@acme.com', displayName: 'Alice Test' });
    await mockHome(page);
    await loginToDashboard(page, { email: 'alice@acme.com', password: 'password123' });
    await expect(page.getByTestId('home-page')).toBeVisible();
  });

  test('lands on Home, which leads the bottom bar', async ({ page }) => {
    await expect(page).toHaveURL(/#home$/);
    const nav = page.getByRole('navigation', { name: 'Primary navigation' });
    const names = await nav.getByRole('button').evaluateAll((buttons) =>
      buttons.map((button) => button.getAttribute('aria-label')),
    );
    expect(names).toEqual(['Home', 'Chat', 'Discover', 'Notifications', 'Open menu']);
    await expect(nav.getByRole('button', { name: 'Home' })).toHaveAttribute('aria-current', 'page');
    await expect(page.getByTestId('home-today-session')).toContainText('Tempo run');
  });

  test('fits the phone: no sideways scroll, the week strip inside the viewport, 44px day targets', async ({ page }) => {
    const { scrollWidth, clientWidth } = await page.evaluate(() => ({
      scrollWidth: document.documentElement.scrollWidth,
      clientWidth: document.documentElement.clientWidth,
    }));
    expect(scrollWidth).toBeLessThanOrEqual(clientWidth + 1);

    const viewport = page.viewportSize();
    expect(viewport).not.toBeNull();
    const cells = page.getByTestId('home-week').getByRole('button');
    await expect(cells).toHaveCount(7);
    for (const box of await cells.evaluateAll((els) => els.map((el) => el.getBoundingClientRect().toJSON()))) {
      expect(box.left).toBeGreaterThanOrEqual(0);
      expect(box.right).toBeLessThanOrEqual((viewport?.width ?? 0) + 1);
      expect(box.height).toBeGreaterThanOrEqual(44);
    }
  });

  test('an indoor latest activity says it has no track; tapping a row opens the analyze draft', async ({ page }) => {
    await expect(page.getByTestId('home-activity-latest')).toContainText('This activity recorded no GPS track.');
    await expect(page.getByTestId('route-sketch')).toHaveCount(1);

    await page.getByTestId('home-activity-row').first().getByRole('button').click();
    await expect(page).toHaveURL(/#chat\/conv-home-mobile$/);
    await expect(page.getByPlaceholder('Message Dravr...').first()).toHaveValue(/^Analyze my activity from .+ \(Run\)$/);
  });
});
