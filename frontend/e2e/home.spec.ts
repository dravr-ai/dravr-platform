// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E for the athlete Home — sign-in lands on it, the rail logo leads back, a day drafts a chat, an activity opens its view
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
  // Home's routes carry no title: the row or the view names the activity above the map.
  title: null,
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
  /** The route answers in order, the last repeating; a drawn route when absent. */
  routes?: Array<Record<string, unknown>>;
  /**
   * The answers to the athlete's route retry (`?retry=true`), which the server
   * reads past its stored answer; the plain answers when absent.
   */
  retriedRoutes?: Array<Record<string, unknown>>;
  /**
   * The answers to the athlete's sync retry (`?retry=true`), which the server
   * refreshes past its pause; the plain answers when absent.
   */
  retriedRecent?: Array<Record<string, unknown>>;
}

/**
 * The Home reads and a provider status, registered after the shared mocks so
 * they win. `recent` is a sequence: each read takes the next answer and the
 * last one repeats, so a spec can script a stale answer and its refresh.
 * Returns the recent-activities call counter and the route calls seen.
 */
async function mockHome(page: Page, answers: HomeAnswers = {}) {
  const recent = answers.recent ?? [
    { activities: ACTIVITIES, as_of: '2026-09-24T08:15:00Z', sync_failure: null, stale: false },
  ];
  const routes = answers.routes ?? [{ route: ROUTE, reason: null }];
  const retriedRoutes = answers.retriedRoutes ?? routes;
  const retriedRecent = answers.retriedRecent ?? recent;
  const providers = answers.providers ?? [provider('strava', 'Strava', { connected: answers.connected ?? true })];
  const calls = { recent: 0, retriedRecent: 0, routes: [] as string[] };
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
    const retried = new URL(route.request().url()).searchParams.get('retry') === 'true';
    const body = retried
      ? retriedRecent[Math.min(calls.retriedRecent, retriedRecent.length - 1)]
      : recent[Math.min(calls.recent, recent.length - 1)];
    if (retried) {
      calls.retriedRecent += 1;
    } else {
      calls.recent += 1;
    }
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });
  });
  await page.route('**/api/me/activities/*/*/route**', async (route) => {
    const url = new URL(route.request().url());
    const retried = url.searchParams.get('retry') === 'true';
    const plainReads = calls.routes.filter((path) => !path.endsWith('?retry=true')).length;
    const retriedReads = calls.routes.length - plainReads;
    const body = retried
      ? retriedRoutes[Math.min(retriedReads, retriedRoutes.length - 1)]
      : routes[Math.min(plainReads, routes.length - 1)];
    calls.routes.push(`${url.pathname}${url.search}`);
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });
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

/** `GET /api/me/activities/strava/act-4` — the Tempo Tuesday row's view, with its splits. */
const TEMPO_DETAIL = {
  activity: ACTIVITIES[1],
  average_heart_rate: 158,
  max_heart_rate: 176,
  average_speed_mps: 2.8333,
  max_speed_mps: 4.1,
  average_power: null,
  calories: 712,
  splits: [
    {
      index: 1,
      distance_meters: 1000,
      elapsed_time_seconds: 362,
      moving_time_seconds: 355,
      elevation_difference_meters: 6,
      average_speed_mps: 2.8169,
      average_heart_rate: 151,
    },
    {
      index: 2,
      distance_meters: 1000,
      elapsed_time_seconds: 350,
      moving_time_seconds: null,
      elevation_difference_meters: -4,
      average_speed_mps: 2.8571,
      average_heart_rate: 160,
    },
  ],
  laps: [],
  conversation_id: null,
};

const RECOVERY_REPLY = 'Keep tomorrow easy: 40 minutes in Z1, then strides.';

/**
 * The activity reads and the chat turn its questions go out on. Every
 * activity's view answers with `details[id]`, or 404 when the spec gives
 * none, naming the thread its view linked (`links`), as the server's detail
 * does; each turn is recorded and answered with {@link RECOVERY_REPLY}, and a
 * read of the thread afterwards holds every turn sent, as the server's does.
 */
async function mockActivityView(page: Page, details: Record<string, unknown> = { 'act-4': TEMPO_DETAIL }) {
  const sent: string[] = [];
  const links: Record<string, string | null> = {};
  const transcript: Array<Record<string, unknown>> = [];
  await page.route(/\/api\/me\/activities\/[^/]+\/[^/?]+(\?.*)?$/, async (route) => {
    const id = decodeURIComponent(new URL(route.request().url()).pathname.split('/').pop() ?? '');
    if (id === 'recent') {
      await route.fallback();
      return;
    }
    const body = details[id];
    await route.fulfill(
      body === undefined
        ? { status: 404, contentType: 'application/json', body: JSON.stringify({ message: 'not found' }) }
        : {
            status: 200,
            contentType: 'application/json',
            body: JSON.stringify({ ...(body as object), conversation_id: links[id] ?? null }),
          },
    );
  });
  await page.route(/\/api\/me\/activities\/[^/]+\/[^/]+\/conversation$/, async (route, request) => {
    const segments = new URL(request.url()).pathname.split('/');
    const id = decodeURIComponent(segments[segments.length - 2] ?? '');
    const link = JSON.parse(request.postData() ?? '{}') as { conversation_id: string | null };
    links[id] = link.conversation_id;
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(link) });
  });
  await page.route(`**/api/chat/conversations/${CONVERSATION.id}/messages**`, async (route, request) => {
    if (request.method() !== 'POST') {
      if (transcript.length === 0) {
        await route.fallback();
        return;
      }
      await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ messages: transcript }) });
      return;
    }
    const content = (JSON.parse(request.postData() ?? '{}') as { content: string }).content;
    sent.push(content);
    const n = sent.length;
    const turn = {
      turn_id: '00000000-0000-4000-8000-000000000676',
      user_message: {
        id: `msg-user-676-${n}`,
        conversation_id: CONVERSATION.id,
        role: 'user',
        content,
        created_at: '2026-09-24T10:05:00Z',
      },
      assistant: {
        message: {
          id: `msg-assistant-676-${n}`,
          conversation_id: CONVERSATION.id,
          role: 'assistant',
          content: RECOVERY_REPLY,
          created_at: '2026-09-24T10:05:02Z',
        },
        blocks: [{ type: 'prose', text: RECOVERY_REPLY }],
        finish_reason: 'stop',
      },
      conversation_updated_at: '2026-09-24T10:05:02Z',
      telemetry: { model: 'test', provider_name: 'test', tool_calls_count: 0, tools_called: [], execution_time_ms: 10 },
    };
    transcript.push(turn.user_message, turn.assistant.message);
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(turn) });
  });
  return Object.assign(sent, { links });
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

    await expect(page.getByRole('figure', { name: 'Map of the recorded route' })).toBeVisible();
    await expect(page.getByTestId('home-activity-row')).toHaveCount(4);
    // The polyline row and the two rows the route endpoint answers for.
    await expect(page.getByTestId('route-sketch')).toHaveCount(3);
    // The latest and the two rows without a polyline — never the row with
    // one, nor the row whose route held no GPS.
    await expect.poll(() => [...calls.routes].sort()).toEqual([
      '/api/me/activities/garmin/act-1/route?burst=true',
      '/api/me/activities/strava/act-3/route?burst=true',
      '/api/me/activities/strava/act-5/route?burst=true',
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

  test("tapping an activity opens its view: the map, its figures and splits, then a chat whose question goes out and is answered there", async ({ page }) => {
    await signInAthlete(page);
    await mockHome(page);
    await mockConversationCreate(page);
    const sent = await mockActivityView(page);
    const created: string[] = [];
    page.on('request', (request) => {
      if (request.method() === 'POST' && /\/api\/chat\/conversations(\?.*)?$/.test(request.url())) {
        created.push(request.url());
      }
    });
    await login(page);

    await page.getByTestId('home-activity-row').first().getByRole('button').click();
    await expect(page).toHaveURL(/#home\/activity\/strava\/act-4$/);
    const view = page.getByTestId('activity-view');
    await expect(view.getByTestId('activity-title')).toHaveText('Tempo Tuesday');
    await expect(view.getByRole('figure', { name: 'Map of the recorded route' })).toBeVisible();
    await expect(view.getByTestId('activity-figure-distance')).toContainText('10.20 km');
    await expect(view.getByTestId('activity-figure-duration')).toContainText('1h');
    await expect(view.getByTestId('activity-figure-average_speed')).toContainText('5:53 /km');
    await expect(view.getByTestId('activity-figure-average_heart_rate')).toContainText('158 bpm');
    await expect(view.getByTestId('activity-figure-calories')).toContainText('712 kcal');
    await expect(view.getByTestId('activity-figure-average_power')).toHaveCount(0);
    await expect(view.getByTestId('activity-splits').getByRole('row')).toHaveCount(3);
    await expect(view.getByTestId('activity-prompts').getByRole('button')).toHaveText([
      'Analyze this effort',
      'Compare with my recent ones',
      'Recovery advice',
      'What to adjust',
    ]);

    await view.getByTestId('activity-prompt-recovery').click();
    // The question and its reply land in the view's own chat, under the questions.
    const chat = view.getByTestId('embedded-chat');
    await expect(chat.getByText(RECOVERY_REPLY)).toBeVisible();
    await expect(chat.getByText(/^What recovery do you advise after my activity “Tempo Tuesday” from .+\?$/)).toBeVisible();
    expect(sent).toHaveLength(1);
    expect(sent[0]).toMatch(/^What recovery do you advise after my activity “Tempo Tuesday” from .+\?$/);
    await expect(page).toHaveURL(/#home\/activity\/strava\/act-4$/);

    // Back to Home and into the same activity again: its thread comes back
    // with it, and the next question goes to that thread, not a new one.
    await view.getByTestId('activity-back').click();
    await expect(page.getByTestId('home-page')).toBeVisible();
    await page.getByTestId('home-activity-row').first().getByRole('button').click();
    await expect(page).toHaveURL(/#home\/activity\/strava\/act-4$/);
    const reopened = page.getByTestId('activity-view').getByTestId('embedded-chat');
    await expect(reopened.getByText(/^What recovery do you advise after my activity “Tempo Tuesday” from .+\?$/)).toBeVisible();
    await expect(reopened.getByText(RECOVERY_REPLY)).toBeVisible();
    await page.getByTestId('activity-prompt-adjust').click();
    await expect.poll(() => sent.length).toBe(2);
    expect(created).toHaveLength(1);
    // The thread is linked to the activity on the server, not only in this
    // page's memory: a fresh load of the app — nothing cached, as on another
    // device — opens the same thread, and a question goes to it.
    expect(sent.links['act-4']).toBe(CONVERSATION.id);
    await login(page);
    await page.getByTestId('home-activity-row').first().getByRole('button').click();
    const afterReload = page.getByTestId('activity-view').getByTestId('embedded-chat');
    await expect(afterReload.getByText(/^What recovery do you advise after my activity “Tempo Tuesday” from .+\?$/)).toBeVisible();
    await page.getByTestId('activity-prompt-analyze').click();
    await expect.poll(() => sent.length).toBe(3);
    expect(created).toHaveLength(1);

    await page.goBack();
    await expect(page).toHaveURL(/#home$/);
    await expect(page.getByTestId('home-page')).toBeVisible();
  });

  test('the latest card opens its view too, and a deep link to an activity the athlete does not hold says so', async ({ page }) => {
    await signInAthlete(page);
    await mockHome(page);
    await mockActivityView(page, { 'act-5': { ...TEMPO_DETAIL, activity: ACTIVITIES[0], splits: [] } });
    await login(page);

    await page.getByTestId('home-activity-latest').getByRole('button', { name: /Long ride/ }).click();
    await expect(page).toHaveURL(/#home\/activity\/strava\/act-5$/);
    await expect(page.getByTestId('activity-title')).toHaveText('Long ride');
    // A ride reads a speed, not a pace.
    await expect(page.getByTestId('activity-figure-average_speed')).toContainText('10.2 km/h');
    await expect(page.getByTestId('activity-splits')).toHaveCount(0);

    await page.goto('/#home/activity/strava/not-mine');
    await expect(page.getByTestId('activity-not-found')).toContainText("This activity isn't among your synced activities.");
    await page.getByTestId('activity-back').click();
    await expect(page).toHaveURL(/#home$/);
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

  test('the strips above the dashboard start right of the rail, so none of their text runs under it', async ({ page, context }) => {
    await signInAthlete(page);
    await mockHome(page);
    await mockConversationCreate(page);
    await login(page);
    await expect(page.getByTestId('home-activity-row')).toHaveCount(4);

    const railRight = async () => {
      const rail = await page.getByTestId('icon-rail').boundingBox();
      expect(rail).not.toBeNull();
      return (rail?.x ?? 0) + (rail?.width ?? 0);
    };
    const startsAfterRail = async (testId: string) => {
      const strip = await page.getByTestId(testId).boundingBox();
      expect(strip).not.toBeNull();
      expect(strip?.x ?? 0).toBeGreaterThanOrEqual(await railRight());
    };

    // Chromium offers an install the page defers; the banner answers it.
    await page.evaluate(() => window.dispatchEvent(new Event('beforeinstallprompt')));
    await expect(page.getByTestId('install-banner')).toBeVisible();
    await startsAfterRail('install-banner');

    await context.setOffline(true);
    await expect(page.getByTestId('offline-banner')).toBeVisible();
    await startsAfterRail('offline-banner');
    await context.setOffline(false);
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
    // The list's top row says new activities are on their way, in words a
    // screen reader hears politely, above the rows the cache already holds.
    const fetching = section.getByTestId('home-activities-fetching');
    await expect(fetching).toBeVisible();
    await expect(fetching).toHaveText('Fetching your latest activities…');
    await expect(fetching).toHaveAttribute('role', 'status');
    await expect(fetching).toHaveAttribute('aria-live', 'polite');
    await expect(section.getByRole('listitem').first()).toContainText('Fetching your latest activities…');
    await expect(latest).toContainText('Long ride');
    expect(calls.recent).toBe(1);

    const [first, second, ...unused] = HOME_STALE_REFETCH_DELAYS_MS;

    // First follow-up: still stale, so the page keeps saying it is checking.
    await page.clock.fastForward(first);
    await expect(latest).toContainText('Lunch run');
    await expect(fetching).toBeVisible();
    expect(calls.recent).toBe(2);

    // Second follow-up, one wider delay on: the fresh answer ends the
    // schedule, and its rows take the fetching row's place.
    await page.clock.fastForward(second);
    await expect(latest).toContainText('Dawn ride');
    await expect(section).toContainText('Last synced:');
    await expect(fetching).toHaveCount(0);
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
    const fetching = section.getByTestId('home-activities-fetching');
    await expect(latest).toContainText('Long ride, read 1');
    await expect(fetching).toBeVisible();

    for (const [index, delay] of delays.entries()) {
      // Halfway through its delay the ask has not gone out. The page's clock
      // also runs on its own between the jumps, so the spec stops well short
      // of the delay; the hook's own test pins it to the millisecond.
      await page.clock.fastForward(delay / 2);
      expect(calls.recent).toBe(index + 1);
      await expect(fetching).toBeVisible();
      // The athlete is still here: a pointer move keeps the idle watch from
      // holding the ask back.
      await page.mouse.move(40 + index, 40);
      await page.clock.fastForward(delay / 2);
      await expect(latest).toContainText(`Long ride, read ${index + 2}`);
      expect(calls.recent).toBe(index + 2);
    }

    // The schedule ended on a stale answer: the rows stay, with their sync time.
    await expect(section).toContainText('Last synced:');
    await expect(fetching).toHaveCount(0);
    await expect(page.getByTestId('home-activity-row')).toHaveCount(4);

    // Never a poll: however long the page stays open, nothing more is asked.
    await page.clock.fastForward(delays[delays.length - 1]);
    await expect(section).toContainText('Last synced:');
    expect(calls.recent).toBe(delays.length + 1);
  });

  // The server's contract after a failed sync: while the failing provider is
  // paused its answer is not stale and names the failure; the retry's own
  // answer is stale — the refresh it started is running — and still names
  // it; the schedule's first follow-up, one delay after the retry, brings the
  // fresh answer that clears it.
  test.describe('in UTC, so the sync time on the card is the one the answer carries', () => {
    test.use({ timezoneId: 'UTC' });

    test('a failed sync names its provider and last good sync once, and a retry clears it on the schedule', async ({ page }) => {
      await page.clock.install({ time: new Date('2026-09-24T12:10:00Z') });
      await signInAthlete(page);
      const failure = {
        provider: 'strava',
        provider_name: 'Strava',
        failed_at: '2026-09-24T12:04:00Z',
        last_synced_at: '2026-09-24T06:15:00Z',
      };
      const calls = await mockHome(page, {
        recent: [
          { activities: ACTIVITIES, as_of: '2026-09-24T08:15:00Z', sync_failure: failure, stale: false },
          { activities: ACTIVITIES, as_of: '2026-09-24T12:30:00Z', sync_failure: null, stale: false },
        ],
        retriedRecent: [{ activities: ACTIVITIES, as_of: '2026-09-24T08:15:00Z', sync_failure: failure, stale: true }],
      });
      await login(page);

      const section = page.getByTestId('home-activities');
      const failed = page.getByTestId('home-sync-failed');
      const fetching = section.getByTestId('home-activities-fetching');
      await expect(failed).toBeVisible();
      // The failure and the fetching row are never on the card together.
      await expect(fetching).toHaveCount(0);
      await expect(failed).toHaveAttribute('role', 'alert');
      await expect(failed).toContainText('Strava · Sync failed');
      await expect(failed).not.toContainText('Last synced');
      // One time on the card: Strava's own last good sync, not the newest sync of any provider.
      await expect(section.getByText(/Last synced:/)).toHaveCount(1);
      await expect(section.getByText(/Last synced:/)).toContainText('06:15');
      await expect(page.getByTestId('home-activity-row')).toHaveCount(4);
      expect(calls.recent).toBe(1);

      await failed.getByTestId('home-sync-retry').click();
      await expect.poll(() => calls.retriedRecent).toBe(1);
      await expect(fetching).toBeVisible();
      await expect(failed).toHaveCount(0);

      await page.mouse.move(40, 40);
      await page.clock.fastForward(HOME_STALE_REFETCH_DELAYS_MS[0]);
      await expect.poll(() => calls.recent).toBe(2);
      await expect(failed).toHaveCount(0);
      await expect(section).toContainText('Last synced:');
      await expect(fetching).toHaveCount(0);
    });
  });

  // The server keeps an `unavailable` answer for ten minutes: a plain re-ask
  // gets it again, and only the retry, which it reads past, can draw.
  test('a route the server could not read says so and draws the map once a retry reads past it', async ({ page }) => {
    await signInAthlete(page);
    const calls = await mockHome(page, {
      routes: [{ route: null, reason: 'unavailable' }],
      retriedRoutes: [{ route: ROUTE, reason: null }],
    });
    await login(page);

    const latest = page.getByTestId('home-activity-latest');
    const failed = latest.getByTestId('home-route-failed');
    await expect(failed).toContainText("The map couldn't be loaded.");
    await expect(latest).not.toContainText('Loading the map…');
    await expect(latest).not.toContainText('This activity recorded no GPS track.');

    await failed.getByRole('button', { name: 'Retry' }).click();
    await expect(page.getByRole('figure', { name: 'Map of the recorded route' })).toBeVisible();
    expect(calls.routes.filter((path) => path.endsWith('/act-5/route?retry=true'))).toHaveLength(1);
  });

  test('a retried route the provider still cannot serve keeps saying the map could not be loaded', async ({ page }) => {
    await signInAthlete(page);
    const calls = await mockHome(page, { routes: [{ route: null, reason: 'unavailable' }] });
    await login(page);

    const latest = page.getByTestId('home-activity-latest');
    const failed = latest.getByTestId('home-route-failed');
    await expect(failed).toBeVisible();
    await failed.getByRole('button', { name: 'Retry' }).click();
    await expect.poll(() => calls.routes.filter((path) => path.endsWith('?retry=true')).length).toBe(1);
    await expect(latest.getByTestId('home-route-failed')).toContainText("The map couldn't be loaded.");
    await expect(latest).not.toContainText('This activity recorded no GPS track.');
  });
});
