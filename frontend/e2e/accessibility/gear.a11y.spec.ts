// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: WCAG 2.1 AA coverage for the gear ledger — Settings › Shoes and an activity's shoe picker — with colour contrast enabled
// ABOUTME: Scans a pair past its warning, the default and pinned badges, a retired pair and the picker's default note, in both themes

import { test, expect, type Page, type Route } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { setupDashboardMocks, loginToDashboard } from '../test-helpers';

const ROAD = '11111111-1111-4111-8111-111111111111';
const TRAIL = '22222222-2222-4222-8222-222222222222';
const OLD = '33333333-3333-4333-8333-333333333333';

function pair(id: string, overrides: Record<string, unknown>) {
  return {
    id,
    kind: 'shoe',
    name: 'Pair',
    sport: 'run',
    initial_distance_meters: 0,
    warn_distance_meters: 500_000,
    distance_meters: 0,
    activity_count: 0,
    warn_reached: false,
    pinned: false,
    is_default: false,
    added_at: '2026-08-01T07:00:00Z',
    retired_at: null,
    last_used_at: null,
    ...overrides,
  };
}

const LEDGER = {
  gear: [
    pair(ROAD, {
      name: 'Pegasus 41',
      distance_meters: 512_300,
      activity_count: 41,
      warn_reached: true,
      is_default: true,
      last_used_at: '2026-10-08T06:30:00Z',
    }),
    pair(TRAIL, { name: 'Speedgoat 6', sport: 'trail_running', pinned: true, is_default: true }),
    pair(OLD, { name: 'Old Clifton', distance_meters: 820_000, activity_count: 90, retired_at: '2026-09-20T10:00:00Z' }),
  ],
};

const RUN_DETAIL = {
  activity: {
    id: 'run-1',
    provider: 'strava',
    name: 'Morning run',
    sport_type: 'run',
    start_date: '2026-10-08T06:30:00Z',
    duration_seconds: 3000,
    distance_meters: 10_000,
    elevation_gain_meters: 40,
    has_gps: false,
    summary_polyline: null,
    attribution: null,
  },
  average_heart_rate: 148,
  max_heart_rate: 166,
  average_speed_mps: 3.33,
  max_speed_mps: null,
  average_power: null,
  calories: 640,
  splits: [],
  laps: [],
  conversation_id: null,
};

const json = (route: Route, body: unknown) =>
  route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });

async function signIn(page: Page) {
  await setupDashboardMocks(page, { role: 'user', email: 'alice@acme.com', displayName: 'Alice Test' });
  await page.route('**/api/me/gear', (route) => json(route, LEDGER));
  await page.route('**/api/me/activities/strava/run-1', (route) => json(route, RUN_DETAIL));
  await page.route('**/api/me/activities/strava/run-1/gear', (route) =>
    json(route, { gear_id: ROAD, source: 'default' }),
  );
  await loginToDashboard(page);
}

/** Colour contrast stays ON; the scan covers the gear surface itself, not the shell around it. */
async function scan(page: Page, testId: string) {
  const results = await new AxeBuilder({ page })
    .withTags(['wcag2a', 'wcag2aa', 'wcag21aa'])
    .include(`[data-testid="${testId}"]`)
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

test.describe('Gear ledger accessibility', () => {
  for (const theme of ['light', 'dark'] as const) {
    test(`Settings › Shoes has no WCAG 2.1 AA violations (${theme})`, async ({ page }) => {
      await signIn(page);
      await page.goto('/#settings/gear');
      await expect(page.getByTestId(`gear-warning-${ROAD}`)).toBeVisible();
      await expect(page.getByTestId('gear-retired')).toBeVisible();
      await setTheme(page, theme);

      expect(await scan(page, 'gear-settings')).toBe('');
    });

    test(`an activity's shoe picker has no WCAG 2.1 AA violations (${theme})`, async ({ page }) => {
      await signIn(page);
      await page.goto('/#home/activity/strava/run-1');
      await expect(page.getByTestId('activity-gear')).toContainText('Counted on the pair you use for this sport.');
      await expect(page.getByTestId(`gear-warning-${ROAD}`)).toBeVisible();
      await setTheme(page, theme);

      expect(await scan(page, 'activity-gear')).toBe('');
    });
  }
});
