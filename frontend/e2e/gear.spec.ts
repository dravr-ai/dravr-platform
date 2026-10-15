// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E for the gear ledger — Settings › Shoes declares and retires pairs, an activity's view records the pair it was run in
// ABOUTME: Runs against a mocked /api/me/gear shaped like the server's, holding the athlete's pairs between requests (carnet#711)

import { test, expect, type Page, type Route } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard } from './test-helpers';

const ROAD = '11111111-1111-4111-8111-111111111111';
const TRAIL = '22222222-2222-4222-8222-222222222222';
const CREATED = '33333333-3333-4333-8333-333333333333';

interface Pair {
  id: string;
  kind: 'shoe';
  name: string;
  sport: string;
  initial_distance_meters: number;
  warn_distance_meters: number;
  distance_meters: number;
  activity_count: number;
  warn_reached: boolean;
  pinned: boolean;
  is_default: boolean;
  added_at: string;
  retired_at: string | null;
  last_used_at: string | null;
}

function pair(id: string, overrides: Partial<Pair>): Pair {
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

const json = (route: Route, body: unknown, status = 200) =>
  route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });

/**
 * The athlete's ledger as the server holds it: each request reads and
 * changes the same pairs, and every request body sent is recorded.
 */
async function mockLedger(page: Page, initial: Pair[]) {
  const state = { gear: initial, sent: [] as Array<{ method: string; path: string; body: unknown }> };
  await page.route('**/api/me/gear', async (route, request) => {
    if (request.method() === 'POST') {
      const body = JSON.parse(request.postData() ?? '{}') as Partial<Pair>;
      state.sent.push({ method: 'POST', path: '/api/me/gear', body });
      const created = pair(CREATED, {
        ...body,
        distance_meters: body.initial_distance_meters ?? 0,
        added_at: '2026-10-09T08:00:00Z',
      });
      state.gear = [created, ...state.gear];
      await json(route, created, 201);
      return;
    }
    await json(route, { gear: state.gear });
  });
  await page.route('**/api/me/gear/*/retire', async (route, request) => {
    const id = new URL(request.url()).pathname.split('/').slice(-2)[0];
    state.sent.push({ method: 'POST', path: `/api/me/gear/${id}/retire`, body: null });
    state.gear = state.gear.map((p) =>
      p.id === id ? { ...p, retired_at: '2026-10-09T09:00:00Z', is_default: false } : p,
    );
    await json(route, state.gear.find((p) => p.id === id));
  });
  return state;
}

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
    attribution: 'COROS PACE 3',
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

async function signIn(page: Page) {
  await setupDashboardMocks(page, { role: 'user', email: 'alice@acme.com', displayName: 'Alice Test' });
}

test.describe('Gear ledger', () => {
  test('Settings › Shoes shows each pair with its distance and warning, declares a pair and retires one with its distance kept', async ({ page }) => {
    await signIn(page);
    const ledger = await mockLedger(page, [
      pair(ROAD, {
        name: 'Pegasus 41',
        distance_meters: 512_300,
        activity_count: 41,
        warn_reached: true,
        is_default: true,
        last_used_at: '2026-10-08T06:30:00Z',
      }),
    ]);
    await loginToDashboard(page);
    await page.goto('/#settings/gear');

    const pane = page.getByTestId('gear-settings');
    const road = pane.getByTestId(`gear-${ROAD}`);
    await expect(road.getByTestId(`gear-distance-${ROAD}`)).toHaveText('512 km');
    await expect(road.getByTestId(`gear-default-${ROAD}`)).toHaveText('Default');
    await expect(road.getByTestId(`gear-warning-${ROAD}`)).toHaveText(
      'This pair has 512 km, past the 500 km you set.',
    );

    await pane.getByTestId('gear-add-name').fill('Speedgoat 6');
    await pane.getByTestId('gear-add-sport').selectOption('trail_running');
    await pane.getByTestId('gear-add-initial').fill('80');
    await pane.getByTestId('gear-add-submit').click();
    await expect(pane.getByTestId(`gear-${CREATED}`)).toContainText('Speedgoat 6');
    await expect(pane.getByTestId(`gear-distance-${CREATED}`)).toHaveText('80 km');
    expect(ledger.sent[0]).toEqual({
      method: 'POST',
      path: '/api/me/gear',
      body: {
        name: 'Speedgoat 6',
        sport: 'trail_running',
        initial_distance_meters: 80_000,
        warn_distance_meters: 500_000,
        pinned: false,
      },
    });

    await road.getByTestId(`gear-retire-${ROAD}`).click();
    const retired = pane.getByTestId('gear-retired');
    await expect(retired.getByTestId(`gear-distance-${ROAD}`)).toHaveText('512 km');
    await expect(pane.getByTestId('gear-in-use').getByTestId(`gear-${ROAD}`)).toHaveCount(0);
  });

  test('Settings › Shoes reads and takes miles for an athlete who reads imperial (carnet#835)', async ({ page }) => {
    await signIn(page);
    // Registered after the shared stubs, so this answer is the one served.
    await page.route('**/api/me/units**', (route) =>
      json(route, { units: 'imperial' }),
    );
    const ledger = await mockLedger(page, [
      pair(ROAD, { name: 'Pegasus 41', distance_meters: 512_300, activity_count: 41, warn_reached: true }),
    ]);
    await loginToDashboard(page);
    await page.goto('/#settings/gear');

    const pane = page.getByTestId('gear-settings');
    await expect(pane.getByTestId(`gear-distance-${ROAD}`)).toHaveText('318 mi');
    await expect(pane.getByTestId(`gear-warning-${ROAD}`)).toHaveText('This pair has 318 mi, past the 311 mi you set.');
    await expect(pane.getByLabel('Distance it already has (mi)')).toBeVisible();
    await expect(pane.getByTestId('gear-add-warn')).toHaveValue('310.7');

    await pane.getByTestId('gear-add-name').fill('Novablast 5');
    await pane.getByTestId('gear-add-initial').fill('50');
    await pane.getByTestId('gear-add-submit').click();
    await expect(pane.getByTestId(`gear-${CREATED}`)).toContainText('Novablast 5');
    // The warning left as shown stays the 500 km it stands for.
    expect(ledger.sent[0]?.body).toEqual({
      name: 'Novablast 5',
      sport: 'run',
      initial_distance_meters: 80_467,
      warn_distance_meters: 500_000,
      pinned: false,
    });
  });

  test("an activity's view shows the pair it was counted on by default and records the athlete's choice", async ({ page }) => {
    await signIn(page);
    await mockLedger(page, [
      pair(ROAD, { name: 'Pegasus 41', is_default: true }),
      pair(TRAIL, { name: 'Speedgoat 6', sport: 'trail_running' }),
    ]);
    let attribution: { gear_id: string | null; source: string | null } = { gear_id: ROAD, source: 'default' };
    const chosen: unknown[] = [];
    await page.route('**/api/me/activities/strava/run-1', (route) => json(route, RUN_DETAIL));
    await page.route('**/api/me/activities/strava/run-1/gear', async (route, request) => {
      if (request.method() === 'PUT') {
        const body = JSON.parse(request.postData() ?? '{}') as { gear_id: string | null };
        chosen.push(body);
        attribution = { gear_id: body.gear_id, source: 'declared' };
      }
      await json(route, attribution);
    });
    await loginToDashboard(page);
    await page.goto('/#home/activity/strava/run-1');

    const view = page.getByTestId('activity-view');
    // The recording device, whoever made it (carnet#711).
    await expect(view.getByTestId('activity-attribution')).toHaveText(/COROS PACE 3/);
    const picker = view.getByTestId('activity-gear');
    const select = picker.getByLabel('Shoes');
    await expect(select).toHaveValue(ROAD);
    await expect(picker).toContainText('Counted on the pair you use for this sport.');

    await select.selectOption(TRAIL);
    await expect(select).toHaveValue(TRAIL);
    await expect(picker).not.toContainText('Counted on the pair you use for this sport.');
    expect(chosen).toEqual([{ gear_id: TRAIL }]);

    await select.selectOption('none');
    await expect(select).toHaveValue('none');
    expect(chosen).toEqual([{ gear_id: TRAIL }, { gear_id: null }]);
  });
});
