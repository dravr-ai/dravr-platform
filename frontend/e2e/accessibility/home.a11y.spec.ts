// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: WCAG 2.1 AA coverage for the athlete Home — the page sign-in lands on — with colour contrast enabled
// ABOUTME: Scans the planned week with activities and the empty no-plan state, in both themes

import { test, expect, type Page } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { setupDashboardMocks, loginToDashboard } from '../test-helpers';

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
        { date: '2026-09-21', sport: 'run', workout: 'Easy run', duration_min: 40, intensity: 'Z2', rest: false },
        {
          date: '2026-09-24',
          sport: 'run',
          workout: 'Tempo run',
          duration_min: 50,
          intensity: 'threshold',
          rest: false,
          steps: [{ label: 'Warm-up', duration_seconds: 900, target_zone: 'Z1' }],
          fueling: { carbs_g_per_h: 40, fluid_ml_per_h: 500 },
        },
        { date: '2026-09-25', sport: 'rest', workout: '', intensity: '', rest: true },
      ],
    },
    { week_start: '2026-09-28', focus: 'absorb the block', current: false, days: [] },
  ],
  weeks_deferred: 0,
};

const ACTIVITIES = [
  {
    id: 'act-2',
    provider: 'strava',
    name: 'Long ride',
    sport_type: 'ride',
    start_date: '2026-09-20T13:00:00Z',
    duration_seconds: 13260,
    distance_meters: 92400,
    elevation_gain_meters: 820,
    has_gps: true,
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

const ROUTE = {
  coordinates: [
    [45.5, -73.6],
    [45.51, -73.61],
    [45.52, -73.63],
  ],
  bounds: { min_latitude: 45.5, max_latitude: 45.52, min_longitude: -73.63, max_longitude: -73.6 },
  elevation_meters: null,
  distances_meters: [0, 1400, 3100],
  climbs: [],
  title: 'Long ride',
  source_tool: 'strava',
};

async function signIn(page: Page, plan: unknown) {
  await setupDashboardMocks(page, { role: 'user' });
  await page.route('**/api/providers', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ providers: [{ provider: 'strava', connected: true, status: 'connected' }] }),
    }),
  );
  await page.route('**/api/me/training-plan**', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ plan, today: TODAY }) }),
  );
  await page.route('**/api/me/activities/recent**', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ activities: ACTIVITIES, as_of: '2026-09-24T08:15:00Z', stale: false }),
    }),
  );
  await page.route('**/api/me/activities/*/*/route', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ route: ROUTE, reason: null }) }),
  );
  await page.route(/openfreemap\.org/, (route) => route.abort());
  await loginToDashboard(page);
  await expect(page.getByTestId('home-page')).toBeVisible();
}

/**
 * Colour contrast stays ON. The one region left out is MapLibre's own canvas
 * and control chrome — third-party, left as it ships (DESIGN.md §2 treats it
 * like a provider's brand colour); the figure's caption, distance and source
 * line around it are the page's own and are scanned.
 */
async function scan(page: Page) {
  const results = await new AxeBuilder({ page })
    .withTags(['wcag2a', 'wcag2aa', 'wcag21aa'])
    .exclude('.maplibregl-map')
    .analyze();
  return results.violations
    .map((v) => `${v.id} (${v.impact}): ${v.help}\n${v.nodes.slice(0, 4).map((n) => `    ${n.target.join(' ')}`).join('\n')}`)
    .join('\n\n');
}

async function setTheme(page: Page, theme: 'light' | 'dark') {
  await page.evaluate((t) => document.documentElement.classList.toggle('dark', t === 'dark'), theme);
  // Let the token transitions land before measuring contrast.
  await page.waitForTimeout(1200);
}

test.describe('Home accessibility', () => {
  for (const theme of ['light', 'dark'] as const) {
    test(`the planned week with activities has no WCAG 2.1 AA violations (${theme})`, async ({ page }) => {
      await signIn(page, PLAN);
      await expect(page.getByTestId('home-today-session')).toBeVisible();
      await expect(page.getByTestId('home-activity-row')).toHaveCount(1);
      // Open a day so the reused plan-card row is measured too.
      await page.getByTestId(`home-week-day-${TODAY}`).click();
      await expect(page.getByTestId('home-week-detail')).toBeVisible();
      await setTheme(page, theme);

      expect(await scan(page)).toBe('');
    });

    test(`the no-plan state has no WCAG 2.1 AA violations (${theme})`, async ({ page }) => {
      await signIn(page, null);
      await expect(page.getByTestId('home-plan-empty')).toBeVisible();
      await setTheme(page, theme);

      expect(await scan(page)).toBe('');
    });
  }
});
