// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins Garmin's brand-guideline attribution on every Home view that shows Garmin data, beneath each title, both themes
// ABOUTME: With GARMIN_NOTICE_DIR set it also saves the screenshots Garmin's display notice carries (carnet#1154)

import { test, expect, type Page } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard } from './test-helpers';

const TODAY = '2026-09-24';
const PART = 'Created in part using data provided by Garmin devices.';
const DEVICE = 'Garmin Forerunner 965';

/** A Home row recorded on a Garmin watch, and one on a phone that names no device. */
const ACTIVITIES = [
  {
    id: 'g-1',
    provider: 'garmin',
    name: 'Tempo run',
    sport_type: 'run',
    start_date: `${TODAY}T14:00:00Z`,
    duration_seconds: 3_000,
    distance_meters: 10_200,
    elevation_gain_meters: 45,
    has_gps: false,
    summary_polyline: null,
    attribution: DEVICE,
  },
  {
    id: 's-1',
    provider: 'strava',
    name: 'Easy spin',
    sport_type: 'ride',
    start_date: '2026-09-22T15:00:00Z',
    duration_seconds: 3_600,
    distance_meters: 28_000,
    elevation_gain_meters: 120,
    has_gps: false,
    summary_polyline: null,
    attribution: null,
  },
];

const STATUS = {
  today: TODAY,
  form: { band: 'productive', pct_of_fitness: -12 },
  trend: [
    { date: '2026-09-21', band: 'balanced', pct_of_fitness: -4 },
    { date: '2026-09-22', band: 'fresh', pct_of_fitness: 6 },
    { date: '2026-09-23', band: 'productive', pct_of_fitness: -9 },
    { date: TODAY, band: 'productive', pct_of_fitness: -12 },
  ],
  load_ratio: { ratio: 1.12, acute_days: 7, chronic_days: 28 },
  recovery_days: 0,
  garmin_contribution: 'part',
};

const VOLUME = {
  today: TODAY,
  weeks: [
    {
      week_start: '2026-09-14',
      sports: [{ sport_type: 'run', activities: 3, distance_meters: 31_000, duration_seconds: 9_600, elevation_gain_meters: 140 }],
    },
    {
      week_start: '2026-09-21',
      sports: [
        { sport_type: 'run', activities: 1, distance_meters: 10_200, duration_seconds: 3_000, elevation_gain_meters: 45 },
        { sport_type: 'ride', activities: 1, distance_meters: 28_000, duration_seconds: 3_600, elevation_gain_meters: 120 },
      ],
    },
  ],
  garmin_contribution: 'part',
};

/** Home's reads, registered after the shared mocks so they win. */
async function mockGarminHome(page: Page) {
  const json = (body: unknown) => ({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });
  await page.route('**/api/me/training-plan**', (route) => route.fulfill(json({ plan: null, today: TODAY })));
  await page.route('**/api/me/training-status', (route) => route.fulfill(json(STATUS)));
  await page.route('**/api/me/training-volume', (route) => route.fulfill(json(VOLUME)));
  await page.route('**/api/me/activities/recent**', (route) =>
    route.fulfill(json({ activities: ACTIVITIES, as_of: `${TODAY}T15:00:00Z`, sync_failure: null, stale: false })),
  );
  await page.route('**/api/me/calendar**', (route) => {
    const url = new URL(route.request().url());
    const from = url.searchParams.get('from') ?? TODAY;
    const to = url.searchParams.get('to') ?? TODAY;
    const activities = ACTIVITIES.map((activity) => ({ date: activity.start_date.slice(0, 10), activity })).filter(
      (entry) => entry.date >= from && entry.date <= to,
    );
    return route.fulfill(
      json({
        today: TODAY,
        from,
        to,
        history_start: '2026-01-01',
        activities,
        plan_weeks: null,
        garmin_contribution: activities.length > 0 ? 'part' : null,
      }),
    );
  });
}

async function openHome(page: Page) {
  await setupDashboardMocks(page, { role: 'user', email: 'alice@acme.com', displayName: 'Alice Test' });
  await mockGarminHome(page);
  await loginToDashboard(page);
  await expect(page.getByTestId('home-page')).toBeVisible();
}

const THEMES = ['light', 'dark'] as const;

async function setTheme(page: Page, theme: (typeof THEMES)[number]) {
  // The app switches on a `.dark` class, not prefers-color-scheme.
  await page.evaluate((t) => document.documentElement.classList.toggle('dark', t === 'dark'), theme);
  // Let the token transitions land before a screenshot samples the theme.
  await page.waitForTimeout(800);
}

/** Where the notice's screenshots go; none are taken when it is unset. */
const NOTICE_DIR = process.env.GARMIN_NOTICE_DIR;

async function shoot(page: Page, name: string) {
  if (NOTICE_DIR) {
    await page.screenshot({ path: `${NOTICE_DIR}/${name}.png`, fullPage: true });
  }
}

test.describe('Garmin attribution on Home (carnet#1154)', () => {
  test('every Home view that shows Garmin data names Garmin directly beneath its title', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 1000 });
    await openHome(page);

    for (const id of ['home-status-garmin', 'home-volume-garmin', 'home-week-garmin']) {
      await expect(page.getByTestId(id)).toHaveText(PART);
    }
    // The activity row names the device beside its title, never in a tooltip.
    await expect(page.getByTestId('activity-attribution').first()).toContainText(DEVICE);

    for (const theme of THEMES) {
      await setTheme(page, theme);
      for (const id of ['home-status-garmin', 'home-volume-garmin', 'home-week-garmin']) {
        await expect(page.getByTestId(id)).toBeVisible();
      }
      await shoot(page, `web-desktop-home-${theme}`);
    }
  });

  test("an open day names each Garmin workout's device beside its title", async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 1000 });
    await openHome(page);

    await page.getByTestId(`home-week-day-${TODAY}`).click();
    await expect(page.getByTestId('calendar-activity-attribution')).toHaveText(` · ${DEVICE}`);
    for (const theme of THEMES) {
      await setTheme(page, theme);
      await shoot(page, `web-desktop-home-day-${theme}`);
    }
  });

  test('the month sheet opens on the Garmin line', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 1000 });
    await openHome(page);

    await page.getByRole('button', { name: 'Month view' }).click();
    await expect(page.getByTestId('home-month-garmin')).toHaveText(PART);
    for (const theme of THEMES) {
      await setTheme(page, theme);
      await shoot(page, `web-desktop-month-${theme}`);
    }
  });

  test("on a phone the folded Today line carries the form's Garmin line", async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await openHome(page);

    await expect(page.getByTestId('home-today-peek-attribution')).toHaveText(PART);
    for (const theme of THEMES) {
      await setTheme(page, theme);
      await shoot(page, `web-phone-home-${theme}`);
    }
  });
});
