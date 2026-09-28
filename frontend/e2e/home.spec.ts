// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E for the athlete Home — sign-in lands on it, the rail logo leads back, and every tap opens a chat draft
// ABOUTME: Runs against mocked /api/me reads shaped like the server's; a stale cache is followed up on a schedule that ends

import { test, expect, type Page } from '@playwright/test';
import { HOME_STALE_REFETCH_DELAYS_MS } from '@pierre/shared-constants';
import { setupDashboardMocks, loginToDashboard } from './test-helpers';

const TODAY = '2026-09-24';

/** A build-phase plan: today (Thursday) is a tempo run, tomorrow a rest day. */
const PLAN = {
  goal_race: { name: 'Montreal Marathon', date: '2026-11-22', discipline: 'run_marathon', priority: 'A' },
  phases: [
    {
      kind: 'build',
      start: '2026-09-14',
      end: '2026-10-26',
      weeks: 6,
      purpose: 'raise the ceiling',
      intent: 'two hard days, the rest easy',
      current: true,
    },
  ],
  current_phase_index: 0,
  weeks: [
    {
      week_start: '2026-09-21',
      focus: 'threshold volume',
      phase_index: 0,
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
          steps: [
            { label: 'Warm-up', duration_seconds: 900, target_zone: 'Z1' },
            { label: 'Tempo', duration_seconds: 1500, target_zone: 'Z3' },
          ],
          fueling: { carbs_g_per_h: 40, fluid_ml_per_h: 500 },
        },
        { date: '2026-09-25', sport: 'rest', workout: '', intensity: '', rest: true },
      ],
    },
    {
      week_start: '2026-09-28',
      focus: 'absorb the block',
      phase_index: 0,
      current: false,
      days: [],
    },
  ],
  weeks_deferred: 4,
};

function activity(id: string, overrides: Record<string, unknown> = {}) {
  return {
    id,
    provider: 'strava',
    name: 'Morning run',
    sport_type: 'run',
    start_date: '2026-09-18T11:00:00Z',
    duration_seconds: 3600,
    distance_meters: 10200,
    elevation_gain_meters: 85,
    has_gps: true,
    summary_polyline: null,
    ...overrides,
  };
}

const ACTIVITIES = [
  activity('act-5', {
    name: 'Long ride',
    sport_type: 'ride',
    start_date: '2026-09-20T13:00:00Z',
    duration_seconds: 13260,
    distance_meters: 92400,
    elevation_gain_meters: 820,
  }),
  activity('act-4', { name: 'Tempo Tuesday', summary_polyline: '_p~iF~ps|U_ulLnnqC_mqNvxq`@' }),
  activity('act-3', { name: 'Hill repeats', start_date: '2026-09-16T11:00:00Z' }),
  // Its route was read once and the recording held no GPS.
  activity('act-2', { name: 'Trainer spin', sport_type: 'virtual_ride', has_gps: false, start_date: '2026-09-15T22:00:00Z' }),
  // Garmin's activity list carries no position and this route was never
  // read: the row says it may have one, and the route endpoint answers.
  activity('act-1', { name: 'Lake loop', provider: 'garmin', has_gps: true, start_date: '2026-09-14T11:00:00Z' }),
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

const CONVERSATION = {
  id: 'conv-home-draft',
  title: 'New conversation',
  agent_id: null,
  created_at: '2026-09-24T10:00:00Z',
  updated_at: '2026-09-24T10:00:00Z',
  message_count: 0,
  unread_count: 0,
  last_message: null,
};

/** One provider of `GET /api/providers`: connected and healthy unless the spec says otherwise. */
function provider(slug: string, displayName: string, overrides: Record<string, unknown> = {}) {
  return {
    provider: slug,
    display_name: displayName,
    requires_oauth: true,
    connected: true,
    needs_reauth: false,
    capabilities: ['activities'],
    consent_required: false,
    ...overrides,
  };
}

interface HomeAnswers {
  plan?: unknown;
  recent?: Array<Record<string, unknown>>;
  connected?: boolean;
  /** The whole provider list, for a spec that needs more than one Strava. */
  providers?: Array<Record<string, unknown>>;
}

/**
 * The Home reads and a provider status, registered after the shared mocks so
 * they win. `recent` is a sequence: each read takes the next answer and the
 * last one repeats, so a spec can script a stale answer and its refresh.
 * Returns the recent-activities call counter and the route calls seen.
 */
async function mockHome(page: Page, answers: HomeAnswers = {}) {
  const recent = answers.recent ?? [{ activities: ACTIVITIES, as_of: '2026-09-24T08:15:00Z', stale: false }];
  const providers = answers.providers ?? [provider('strava', 'Strava', { connected: answers.connected ?? true })];
  const calls = { recent: 0, routes: [] as string[] };
  await page.route('**/api/providers', async (route) => {
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ providers }) });
  });
  await page.route('**/api/me/training-plan**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ plan: answers.plan === undefined ? PLAN : answers.plan, today: TODAY }),
    });
  });
  await page.route('**/api/me/activities/recent**', async (route) => {
    const body = recent[Math.min(calls.recent, recent.length - 1)];
    calls.recent += 1;
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });
  });
  await page.route('**/api/me/activities/*/*/route', async (route) => {
    calls.routes.push(new URL(route.request().url()).pathname);
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ route: ROUTE, reason: null }),
    });
  });
  // Keep the map hermetic: the basemap is a third-party tile server, and the
  // figure, its caption and the source line are what the page owns.
  await page.route(/openfreemap\.org/, (route) => route.abort());
  return calls;
}

/** The conversation a draft opens, and its empty transcript. */
async function mockConversationCreate(page: Page) {
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

async function signInAthlete(page: Page) {
  await setupDashboardMocks(page, { role: 'user', email: 'alice@acme.com', displayName: 'Alice Test' });
}

async function login(page: Page) {
  await loginToDashboard(page, { email: 'alice@acme.com', password: 'password123' });
  await expect(page.getByTestId('home-page')).toBeVisible();
}

test.describe('Athlete Home', () => {
  test('sign-in lands on Home: today, the week, and the latest activities', async ({ page }) => {
    await signInAthlete(page);
    const calls = await mockHome(page);
    await login(page);

    await expect(page).toHaveURL(/#home$/);
    await expect(page.getByRole('heading', { level: 2, name: 'Home' })).toBeVisible();
    await expect(page.getByTestId('home-today-session')).toContainText('Tempo run');
    await expect(page.getByTestId('home-today-session')).toContainText('Build · week 2');
    await expect(page.getByTestId('home-tomorrow')).toContainText('Rest');
    await expect(page.getByTestId(`home-week-day-${TODAY}`)).toHaveAttribute('aria-current', 'date');
    await expect(page.getByTestId('home-next-week')).toContainText('absorb the block');

    await expect(page.getByRole('figure', { name: 'Map of the recorded route: Long ride' })).toBeVisible();
    await expect(page.getByTestId('home-activity-row')).toHaveCount(4);
    // The polyline row and the two rows the route endpoint answers for.
    await expect(page.getByTestId('route-sketch')).toHaveCount(3);
    // The latest and the two rows without a polyline — never the row with
    // one, nor the row whose route held no GPS.
    await expect.poll(() => [...calls.routes].sort()).toEqual([
      '/api/me/activities/garmin/act-1/route',
      '/api/me/activities/strava/act-3/route',
      '/api/me/activities/strava/act-5/route',
    ]);
    await expect(page.getByTestId('provider-reconnect-banner')).toHaveCount(0);
    // The rail marks Home as the page the athlete is on.
    await expect(page.getByTestId('icon-rail').getByRole('button', { name: 'Home', exact: true }).last()).toHaveAttribute(
      'aria-current',
      'page',
    );
  });

  test('the rail logo leads back to Home from anywhere', async ({ page }) => {
    await signInAthlete(page);
    await mockHome(page);
    await login(page);

    await page.getByTestId('icon-rail').getByRole('button', { name: 'Chat', exact: true }).click();
    await expect(page).toHaveURL(/#chat$/);
    await expect(page.getByTestId('home-page')).toHaveCount(0);

    await page.getByTestId('rail-logo-home').click();
    await expect(page).toHaveURL(/#home$/);
    await expect(page.getByTestId('home-page')).toBeVisible();
  });

  test('no plan offers one "Build my plan" button, which opens chat with the request drafted', async ({ page }) => {
    await signInAthlete(page);
    await mockHome(page, { plan: null });
    await mockConversationCreate(page);
    await login(page);

    const empty = page.getByTestId('home-plan-empty');
    await expect(empty).toContainText('No training plan yet');
    await expect(page.getByTestId('home-page').locator('.btn-primary')).toHaveCount(1);
    await expect(page.getByTestId('home-week')).toHaveCount(0);

    await empty.getByRole('button', { name: 'Build my plan' }).click();
    await expect(page).toHaveURL(/#chat\/conv-home-draft$/);
    await expect(page.getByPlaceholder('Message Dravr...').first()).toHaveValue(
      'Build me a training plan for my goal race.',
    );
  });

  test('tapping an activity opens a new chat with the analyze draft in the composer', async ({ page }) => {
    await signInAthlete(page);
    await mockHome(page);
    await mockConversationCreate(page);
    await login(page);

    await page.getByTestId('home-activity-row').first().getByRole('button').click();
    await expect(page).toHaveURL(/#chat\/conv-home-draft$/);
    await expect(page.getByPlaceholder('Message Dravr...').first()).toHaveValue(
      /^Analyze my activity from .+ \(Run\)$/,
    );
  });

  test('a plan day opens to its steps, and its question drafts in chat', async ({ page }) => {
    await signInAthlete(page);
    await mockHome(page);
    await mockConversationCreate(page);
    await login(page);

    await page.getByTestId(`home-week-day-${TODAY}`).click();
    const detail = page.getByTestId('home-week-detail');
    await expect(detail).toContainText('Warm-up · 15m · Z1');
    await detail.getByRole('button', { name: /^Walk me through my session on .+: Tempo run$/ }).click();
    await expect(page.getByPlaceholder('Message Dravr...').first()).toHaveValue(
      /^Walk me through my session on .+: Tempo run$/,
    );
  });

  test('no provider connected asks for one and leads to the connections pane', async ({ page }) => {
    await signInAthlete(page);
    await mockHome(page, { connected: false, recent: [{ activities: [], as_of: null, stale: false }] });
    await login(page);

    const prompt = page.getByTestId('home-connect-provider');
    await expect(prompt).toContainText('Connect a fitness provider to see your recent activities here.');
    await prompt.getByRole('button', { name: 'Connect' }).click();
    await expect(page).toHaveURL(/#settings\/connections$/);
  });

  test('a connection to reconnect is named by the shell banner on every tab, and leads to the connections pane', async ({ page }) => {
    await signInAthlete(page);
    await mockHome(page, {
      providers: [
        provider('strava', 'Strava'),
        provider('garmin', 'Garmin', { needs_reauth: true }),
        // A disconnected provider is never named, whatever its flag says.
        provider('coros', 'COROS', { connected: false, needs_reauth: true }),
      ],
      recent: [{ activities: ACTIVITIES, as_of: '2026-09-22T06:00:00Z', stale: false }],
    });
    await mockConversationCreate(page);
    await login(page);

    const banner = page.getByTestId('provider-reconnect-banner');
    await expect(banner).toBeVisible();
    await expect(banner).toHaveAttribute('role', 'alert');
    await expect(banner).toContainText('Reconnect needed');
    await expect(banner).toContainText('Reconnect Garmin to see your new activities.');
    await expect(banner).not.toContainText('COROS');
    // It sits above the page, and the card below does not say it again.
    const bannerBox = await banner.boundingBox();
    const homeBox = await page.getByTestId('home-page').boundingBox();
    expect(bannerBox).not.toBeNull();
    expect(homeBox).not.toBeNull();
    expect(bannerBox?.y ?? Infinity).toBeLessThan(homeBox?.y ?? 0);
    await expect(page.getByTestId('home-activities')).not.toContainText('Reconnect');
    await expect(page.getByTestId('home-connect-provider')).toHaveCount(0);
    // What the cache holds stays on the page.
    await expect(page.getByTestId('home-activities')).toContainText('Last synced:');
    await expect(page.getByTestId('home-activity-row')).toHaveCount(4);

    // Another tab: the strip stays.
    await page.getByTestId('icon-rail').getByRole('button', { name: 'Chat', exact: true }).click();
    await expect(page).toHaveURL(/#chat$/);
    await expect(page.getByTestId('home-page')).toHaveCount(0);
    await expect(banner).toBeVisible();
    await expect(banner).toContainText('Reconnect Garmin to see your new activities.');

    await banner.getByRole('button', { name: 'Reconnect' }).click();
    await expect(page).toHaveURL(/#settings\/connections$/);
    // The connections pane still carries it until the connection is renewed.
    await expect(banner).toBeVisible();
  });

  test('a healthy connection raises no reconnect banner on any tab', async ({ page }) => {
    await signInAthlete(page);
    await mockHome(page);
    await mockConversationCreate(page);
    await login(page);

    await expect(page.getByTestId('home-activity-row')).toHaveCount(4);
    await expect(page.getByTestId('provider-reconnect-banner')).toHaveCount(0);
    await page.getByTestId('icon-rail').getByRole('button', { name: 'Chat', exact: true }).click();
    await expect(page).toHaveURL(/#chat$/);
    await expect(page.getByTestId('provider-reconnect-banner')).toHaveCount(0);
  });

  test('a stale cache is asked for again until an answer is fresh, each ask after its own delay', async ({ page }) => {
    await page.clock.install({ time: new Date('2026-09-24T10:00:00Z') });
    await signInAthlete(page);
    // The token provider's refresh lands within seconds; the scraped one
    // takes longer, and the cache stays stale until it has answered too.
    const fromToken = activity('act-6', { name: 'Lunch run', start_date: '2026-09-24T09:00:00Z' });
    const fromScrape = activity('act-7', {
      name: 'Dawn ride',
      provider: 'garmin',
      sport_type: 'ride',
      start_date: '2026-09-24T09:30:00Z',
    });
    const calls = await mockHome(page, {
      recent: [
        { activities: ACTIVITIES, as_of: '2026-09-22T06:00:00Z', stale: true },
        { activities: [fromToken, ...ACTIVITIES.slice(0, 4)], as_of: '2026-09-22T06:00:00Z', stale: true },
        {
          activities: [fromScrape, fromToken, ...ACTIVITIES.slice(0, 3)],
          as_of: '2026-09-24T10:00:40Z',
          stale: false,
        },
      ],
    });
    await login(page);

    const section = page.getByTestId('home-activities');
    const latest = page.getByTestId('home-activity-latest');
    await expect(section).toContainText('Checking your provider for new activities…');
    await expect(latest).toContainText('Long ride');
    expect(calls.recent).toBe(1);

    const [first, second, ...unused] = HOME_STALE_REFETCH_DELAYS_MS;

    // First follow-up: still stale, so the page keeps saying it is checking.
    await page.clock.fastForward(first);
    await expect(latest).toContainText('Lunch run');
    await expect(section).toContainText('Checking your provider for new activities…');
    expect(calls.recent).toBe(2);

    // Second follow-up, one wider delay on: the fresh answer ends the schedule.
    await page.clock.fastForward(second);
    await expect(latest).toContainText('Dawn ride');
    await expect(section).toContainText('Last synced:');
    await expect(section).not.toContainText('Checking your provider for new activities…');
    expect(calls.recent).toBe(3);

    // Never a poll: the delays the schedule had left ask for nothing more.
    for (const delay of unused) {
      await page.clock.fastForward(delay);
    }
    await expect(section).toContainText('Last synced:');
    expect(calls.recent).toBe(3);
  });

  test('a cache that stays stale is asked for once per delay, then the page says when it last synced', async ({ page }) => {
    await page.clock.install({ time: new Date('2026-09-24T10:00:00Z') });
    await signInAthlete(page);
    const delays = HOME_STALE_REFETCH_DELAYS_MS;
    // Every answer is stale. Each one names its latest row after its place in
    // the sequence, so the spec can see the page has taken an answer in
    // before it moves the clock to the next ask.
    const answers = Array.from({ length: delays.length + 1 }, (_, index) => ({
      activities: [{ ...ACTIVITIES[0], name: `Long ride, read ${index + 1}` }, ...ACTIVITIES.slice(1)],
      as_of: '2026-09-22T06:00:00Z',
      stale: true,
    }));
    const calls = await mockHome(page, { recent: answers });
    await login(page);

    const section = page.getByTestId('home-activities');
    const latest = page.getByTestId('home-activity-latest');
    await expect(latest).toContainText('Long ride, read 1');
    await expect(section).toContainText('Checking your provider for new activities…');

    for (const [index, delay] of delays.entries()) {
      // Halfway through its delay the ask has not gone out. The page's clock
      // also runs on its own between the jumps, so the spec stops well short
      // of the delay; the hook's own test pins it to the millisecond.
      await page.clock.fastForward(delay / 2);
      expect(calls.recent).toBe(index + 1);
      await expect(section).toContainText('Checking your provider for new activities…');
      // The athlete is still here: a pointer move keeps the idle watch from
      // holding the ask back.
      await page.mouse.move(40 + index, 40);
      await page.clock.fastForward(delay / 2);
      await expect(latest).toContainText(`Long ride, read ${index + 2}`);
      expect(calls.recent).toBe(index + 2);
    }

    // The schedule ended on a stale answer: the rows stay, with their sync time.
    await expect(section).toContainText('Last synced:');
    await expect(section).not.toContainText('Checking your provider for new activities…');
    await expect(page.getByTestId('home-activity-row')).toHaveCount(4);

    // Never a poll: however long the page stays open, nothing more is asked.
    await page.clock.fastForward(delays[delays.length - 1]);
    await expect(section).toContainText('Last synced:');
    expect(calls.recent).toBe(delays.length + 1);
  });
});
