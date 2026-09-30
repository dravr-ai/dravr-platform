// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Real-backend E2E: Home against a scripted scraper — the 2026-09-29 incident, a restarted or shedding scraper, hung reads, and every retry
// ABOUTME: Connects a fresh athlete's Strava through the real sciotte login, then drives the SPA; only the scraper is a double

import { DatabaseSync } from 'node:sqlite';
import { test, expect, request as apiRequest, type APIRequestContext, type Page } from '@playwright/test';
import {
  SCIOTTE_DOUBLE_PORT,
  startSciotteDouble,
  type ListRead,
  type SciotteDouble,
  type ScrapedRide,
} from './sciotte-double';

// Opt-in real-server spec (`bun run test:e2e:real`). It needs a Pierre
// server started with DRAVR_SCIOTTE_REMOTE_URL=http://127.0.0.1:8097 (the
// port is SCIOTTE_DOUBLE_PORT) — this spec starts the scraper double there
// itself — on the SQLite file E2E_REAL_DATABASE_PATH names, which one case
// writes a past sync time into, plus the SPA in front of it. CI's
// integration workflow starts that stack. Locally, the standard dev stack
// points the server at the real scraper instead, so run
// `scripts/e2e-home-sync-local.sh`: it boots an isolated server and SPA on
// their own ports and database, runs this spec against them, and tears them
// down. Everything between the browser and the scraper is the real product —
// the sciotte credential login and session export, the Home refresh, the
// freshness judgement and its pause, the route read and the map.
const PIERRE_URL = process.env.PIERRE_URL ?? 'http://127.0.0.1:8081';
const FRONTEND_URL = process.env.FRONTEND_URL ?? 'http://localhost:5173';
const ADMIN_EMAIL = process.env.ADMIN_EMAIL ?? 'admin@example.com';
const ADMIN_PASSWORD = process.env.ADMIN_PASSWORD ?? 'AdminPassword123';
const DATABASE_PATH = process.env.E2E_REAL_DATABASE_PATH;
/** How long the server lets a route request wait (`PIERRE_HOME_ROUTE_ANSWER_SECS`, 25 by default). */
const ROUTE_ANSWER_SECS = Number(process.env.PIERRE_HOME_ROUTE_ANSWER_SECS ?? '25');

/** Every onboarding step the web flow would stop the athlete on before Home. */
const ONBOARDING_STEPS = [
  'profile_type',
  'about_you',
  'parq',
  'coach_proposal',
  'messaging_channel',
  'messaging_configure',
] as const;

const TODAYS_RIDE = 'Sortie du jour e2e';
const EVENING_RIDE = 'Sortie du soir e2e';
const NIGHT_RIDE = 'Sortie de nuit e2e';
/** The hermetic basemap's ground colour: a pixel far from it is something the map painted on it. */
const GROUND: [number, number, number] = [0x10, 0x14, 0x18];

/** A basemap with no tiles to fetch: the map loads its style from here, never from the internet. */
const HERMETIC_STYLE = {
  version: 8,
  sources: {},
  layers: [{ id: 'background', type: 'background', paint: { 'background-color': '#101418' } }],
};

/** How the page prints a sync time, in the language it renders. */
const SYNC_TIME = new Intl.DateTimeFormat('en', {
  day: 'numeric',
  month: 'short',
  hour: '2-digit',
  minute: '2-digit',
  hourCycle: 'h23',
});

/**
 * One ride started `hoursAgo`. Two rides whose times overlap are one activity
 * recorded twice to the platform's de-duplication, so the rides a case adds
 * later are short and start after the earlier ones ended.
 */
function ride(id: string, name: string, hoursAgo: number, durationSeconds = 5400): ScrapedRide {
  return {
    id,
    name,
    sport_type: 'ride',
    start_date: new Date(Date.now() - hoursAgo * 3600_000).toISOString(),
    duration_seconds: durationSeconds,
    provider: 'strava',
    distance_meters: 42000,
    elevation_gain: 310,
  };
}

async function accessToken(ctx: APIRequestContext, email: string, password: string): Promise<string> {
  const response = await ctx.post('/oauth/token', {
    headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    form: { grant_type: 'password', username: email, password },
  });
  expect(response.ok(), `login of ${email} failed: ${response.status()} — re-run the setup script`).toBeTruthy();
  const { access_token: token } = await response.json();
  return token as string;
}

/** Register a fresh athlete, approve them as the seeded admin, and return a bearer for them. */
async function freshAthlete(
  ctx: APIRequestContext,
  email: string,
  password: string,
): Promise<{ userId: string; token: string }> {
  const registered = await ctx.post('/api/auth/register', {
    data: { email, password, display_name: 'Home Sync E2E' },
  });
  expect(registered.status(), `register failed: ${registered.status()}`).toBe(201);
  const { user_id: userId, user_status: status } = await registered.json();
  if (status === 'pending') {
    const admin = await accessToken(ctx, ADMIN_EMAIL, ADMIN_PASSWORD);
    const approved = await ctx.post(`/api/admin/approve-user/${userId}`, {
      headers: { Authorization: `Bearer ${admin}` },
      data: { reason: 'e2e home sync' },
    });
    expect(approved.ok(), `approve-user failed: ${approved.status()}`).toBeTruthy();
  }
  return { userId, token: await accessToken(ctx, email, password) };
}

/**
 * The basemap style is served here, so a drawn track does not depend on
 * reaching a tile host; anything else the basemap host would be asked for is
 * refused. One handler, since Playwright runs the last-registered match first.
 */
async function hermeticBasemap(page: Page): Promise<void> {
  await page.route(/openfreemap\.org/, (route) =>
    /\/styles\//.test(route.request().url())
      ? route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(HERMETIC_STYLE) })
      : route.abort(),
  );
}

async function signIn(page: Page, email: string, password: string): Promise<void> {
  // The copy asserted below is English. The suite's storage state pins it for
  // http://localhost:5173 only, and a checkout moved off that port serves the
  // SPA from another origin, so the spec states its language on every origin.
  await page.addInitScript(() => window.localStorage.setItem('pierre_app_language', 'en'));
  await page.goto(FRONTEND_URL);
  await page.locator('input[name="email"]').fill(email);
  await page.locator('input[name="password"]').fill(password);
  await page.locator('form button[type="submit"]').first().click();
}

/** A time in the text form the server writes it. */
function stored(at: Date): string {
  return at.toISOString().replace('Z', '+00:00');
}

/**
 * Move the athlete's last good sync back to `at`, as if it happened then:
 * every cached row's `synced_at` and every fetch mark, and any failure
 * recorded before that sync further back still, so it stays superseded. No
 * request can write a time in the past, so the fixture writes it in the
 * server's database.
 */
function ageLastSync(userId: string, at: Date): void {
  expect(DATABASE_PATH, 'E2E_REAL_DATABASE_PATH names the SQLite file the server under test uses').toBeTruthy();
  const db = new DatabaseSync(DATABASE_PATH as string);
  try {
    // The server holds the same file open; wait out its writes rather than fail on them.
    db.exec('PRAGMA busy_timeout = 5000');
    const rows = db.prepare('UPDATE cached_activities SET synced_at = ? WHERE user_id = ?').run(stored(at), userId);
    const marks = db
      .prepare('UPDATE activity_fetch_freshness SET fetched_at = ? WHERE user_id = ?')
      .run(stored(at), userId);
    db.prepare('UPDATE activity_fetch_failures SET failed_at = ? WHERE user_id = ?').run(
      stored(new Date(at.getTime() - 3600_000)),
      userId,
    );
    expect(Number(rows.changes), 'the athlete has cached rides to age').toBeGreaterThan(0);
    expect(Number(marks.changes), 'the athlete has a fetch mark to age').toBeGreaterThan(0);
  } finally {
    db.close();
  }
}

/** Forget the stored route read of one ride, as if its map had never been read. */
function forgetRoute(userId: string, activityId: string): void {
  expect(DATABASE_PATH, 'E2E_REAL_DATABASE_PATH names the SQLite file the server under test uses').toBeTruthy();
  const db = new DatabaseSync(DATABASE_PATH as string);
  try {
    db.exec('PRAGMA busy_timeout = 5000');
    db.prepare('DELETE FROM activity_route_tracks WHERE user_id = ? AND activity_id = ?').run(userId, activityId);
  } finally {
    db.close();
  }
}

/**
 * How many pixels of the map's left part the basemap did not paint — the
 * track's line and halo, on a ground that is otherwise one colour. The canvas
 * is WebGL, so the pixels come from a screenshot of it, decoded in the page.
 * The zoom controls sit bottom-right, outside the part counted.
 */
async function paintedPixels(page: Page, map: ReturnType<Page['locator']>): Promise<number> {
  const png = (await map.locator('canvas').first().screenshot()).toString('base64');
  return page.evaluate(
    async ({ png, ground }) => {
      const image = new Image();
      image.src = `data:image/png;base64,${png}`;
      await image.decode();
      const canvas = document.createElement('canvas');
      canvas.width = image.width;
      canvas.height = image.height;
      const context = canvas.getContext('2d');
      if (context === null) return 0;
      context.drawImage(image, 0, 0);
      const width = Math.floor(image.width * 0.6);
      const { data } = context.getImageData(0, 0, width, image.height);
      let painted = 0;
      for (let i = 0; i < data.length; i += 4) {
        const distance =
          Math.abs(data[i] - ground[0]) + Math.abs(data[i + 1] - ground[1]) + Math.abs(data[i + 2] - ground[2]);
        if (distance > 90) painted += 1;
      }
      return painted;
    },
    { png, ground: GROUND },
  );
}

/** The latest card's map is drawn: the figure says so, and the track's pixels are on the canvas. */
async function expectDrawnTrack(page: Page, rideName: string): Promise<void> {
  const latest = page.getByTestId('home-activity-latest');
  const map = latest.getByRole('figure', { name: new RegExp(`Map of the recorded route: ${rideName}`) });
  await expect(map).toHaveAttribute('data-route-drawn', 'true', { timeout: 30_000 });
  await expect(latest).not.toContainText('Loading the map…');
  // A frame for the renderer to paint what style.load added.
  await expect.poll(() => paintedPixels(page, map), { timeout: 10_000 }).toBeGreaterThan(200);
}

/** The Home head window a list read asked for: from thirty days back, open-ended, no detail pass. */
function expectHeadWindow(read: ListRead | undefined, askedAt: number): void {
  expect(read, 'a list read reached the scraper').toBeTruthy();
  const query = (read as ListRead).query;
  const after = Number(query.get('after')) * 1000;
  expect(Math.abs(after - (askedAt - 30 * 86_400_000)), query.toString()).toBeLessThan(120_000);
  expect(query.get('before'), query.toString()).toBeNull();
  expect(query.get('detail'), `no detail pass inside Home's list read: ${query.toString()}`).toBeNull();
}

test.describe('Home sync truth — real backend, scripted scraper', () => {
  test.describe.configure({ mode: 'serial' });

  let scraper: SciotteDouble;
  let ctx: APIRequestContext;
  let userId: string;
  let bearer: string;
  const email = `e2e-home-sync-${Date.now()}@example.com`;
  const password = 'HomeSyncPassw0rd!';

  test.beforeAll(async () => {
    scraper = await startSciotteDouble([
      ride('e2e-ride-3', TODAYS_RIDE, 4),
      ride('e2e-ride-2', 'Sortie de mardi e2e', 30),
      ride('e2e-ride-1', 'Sortie de dimanche e2e', 72),
      ride('e2e-ride-x4', 'Sortie x4 e2e', 96),
      ride('e2e-ride-x5', 'Sortie x5 e2e', 120),
      // Past the five Home shows: no page load ever reads their routes, so
      // the API-level cases below make their first reads.
      ride('e2e-ride-0', 'Sortie hors page e2e', 144),
      ride('e2e-ride-00', 'Sortie home trainer e2e', 168),
      ride('e2e-ride-h', 'Sortie bloquée e2e', 192),
    ]);
    ctx = await apiRequest.newContext({ baseURL: PIERRE_URL });
    ({ userId, token: bearer } = await freshAthlete(ctx, email, password));
    const auth = { Authorization: `Bearer ${bearer}` };

    // The scraper is failing from the start: the connect succeeds (the login
    // and the export answer), but every list read fails.
    scraper.list = 'fails';
    const connected = await ctx.post('/api/providers/sciotte/login', {
      headers: auth,
      data: { email: 'athlete@strava.example', password: 'never-real', method: 'email', target: 'strava', tos_consent: true },
    });
    expect(
      connected.ok(),
      `sciotte login failed (${connected.status()}): is the server started with DRAVR_SCIOTTE_REMOTE_URL=http://127.0.0.1:${SCIOTTE_DOUBLE_PORT}? scripts/e2e-home-sync-local.sh starts one that is. ${await connected.text()}`,
    ).toBeTruthy();
    expect((await connected.json()).status).toBe('connected');

    for (const step of ONBOARDING_STEPS) {
      const done = await ctx.put(`/api/me/onboarding/steps/${step}`, { headers: auth, data: { status: 'skipped' } });
      expect(done.ok(), `onboarding step ${step}: ${done.status()}`).toBeTruthy();
    }
  });

  test.afterAll(async () => {
    const admin = await accessToken(ctx, ADMIN_EMAIL, ADMIN_PASSWORD).catch(() => undefined);
    if (admin && userId) {
      const suspended = await ctx.post(`/api/admin/suspend-user/${userId}`, {
        headers: { Authorization: `Bearer ${admin}` },
        data: { reason: 'e2e cleanup: home sync' },
      });
      if (!suspended.ok()) {
        console.warn(`e2e cleanup: failed to suspend ${email} (${suspended.status()})`);
      }
    }
    await ctx?.dispose();
    await scraper?.stop();
  });

  test('a failed scrape is said with a retry that lands the rides, and a map that failed draws on its own retry', async ({
    page,
  }) => {
    // The retry is clicked late in the page's refetch schedule, past its
    // 45 s ask, where a retry that waited on the schedule would sit for 105 s.
    test.setTimeout(240_000);
    await hermeticBasemap(page);

    await signIn(page, email, password);
    const section = page.getByTestId('home-activities');
    const failed = page.getByTestId('home-sync-failed');
    const latest = page.getByTestId('home-activity-latest');
    const loaded = Date.now();

    await test.step('the failed scrape is said, naming the provider, never read as synced', async () => {
      await expect(failed).toBeVisible({ timeout: 45_000 });
      await expect(failed).toHaveAttribute('role', 'alert');
      await expect(failed).toContainText('Strava · Sync failed');
      expect(scraper.listReads()).toBeGreaterThanOrEqual(1);
      await expect(section).not.toContainText(TODAYS_RIDE);

      const recent = await ctx.get('/api/me/activities/recent', { headers: { Authorization: `Bearer ${bearer}` } });
      const body = await recent.json();
      expect(body.sync_failure?.provider, JSON.stringify(body)).toBe('strava');
      expect(body.sync_failure.provider_name).toBe('Strava');
      expect(body.sync_failure.last_synced_at).toBeNull();
      // Paused after the failure: no refresh is running, so the page is not stale.
      expect(body.stale, JSON.stringify(body)).toBe(false);
      await page.screenshot({ path: test.info().outputPath('home-sync-failed.png'), fullPage: true });
    });

    await test.step('the scraper recovers; a retry late in the schedule reads it at once and lands the rides', async () => {
      await page.waitForTimeout(Math.max(0, loaded + 50_000 - Date.now()));
      await expect(failed).toBeVisible();
      scraper.list = 'rows';
      // The route read is still failing: the latest map says so below.
      scraper.detail = 'fails';
      const clicked = Date.now();
      await failed.getByTestId('home-sync-retry').click();
      await expect(failed).toHaveCount(0);
      await expect(section).toContainText(TODAYS_RIDE, { timeout: 20_000 });
      await expect(section).toContainText('Sortie de dimanche e2e');
      await expect(failed).toHaveCount(0);
      const afterClick = scraper.listReadLog().filter((read) => read.at >= clicked);
      expect(afterClick.length, 'the retry itself read the scraper').toBeGreaterThanOrEqual(1);
      expectHeadWindow(afterClick[0], clicked);
    });

    await test.step('the latest map could not be loaded, and its retry draws it once the scraper reads it', async () => {
      const mapFailed = latest.getByTestId('home-route-failed');
      await expect(mapFailed).toContainText("The map couldn't be loaded.", { timeout: 30_000 });
      await expect(latest).not.toContainText('This activity recorded no GPS track.');
      await page.screenshot({ path: test.info().outputPath('home-map-failed.png'), fullPage: true });

      scraper.detail = 'route';
      const readsBefore = scraper.detailReads();
      await mapFailed.getByRole('button', { name: 'Retry' }).click();
      await expectDrawnTrack(page, TODAYS_RIDE);
      expect(scraper.detailReads(), 'the retry read the scraper past the stored answer').toBe(readsBefore + 1);
      await page.screenshot({ path: test.info().outputPath('home-sync-recovered.png'), fullPage: true });
    });
  });

  test('the 2026-09-29 incident: a list scrape and a route read at once, the list dying empty, is a failed sync with the rides and map kept', async ({
    page,
  }) => {
    test.setTimeout(180_000);
    await hermeticBasemap(page);
    const lastGood = new Date(Date.now() - 5 * 3600_000);
    ageLastSync(userId, lastGood);
    // The latest ride's map is unread, so the page reads it while the
    // refresh's list walk is on the same session.
    forgetRoute(userId, 'e2e-ride-3');
    scraper.list = 'dies_under_a_detail';
    scraper.detail = 'route';
    const killedBefore = scraper.walksKilledByADetail();

    const loaded = Date.now();
    await signIn(page, email, password);
    const section = page.getByTestId('home-activities');
    const failed = page.getByTestId('home-sync-failed');

    await test.step('the dead walk is a failed sync, dated from the last good one, with the rides and the map kept', async () => {
      await expect(failed).toBeVisible({ timeout: 45_000 });
      expect(
        scraper.listReadLog().some((read) => read.at >= loaded),
        'the stale head was read, and the scraper answered it empty',
      ).toBe(true);
      expect(
        scraper.walksKilledByADetail(),
        'the list walk and the route read were in flight together, and the route read finished first',
      ).toBeGreaterThan(killedBefore);
      expect(scraper.maxInFlight()).toBeGreaterThanOrEqual(2);
      await expect(failed).toContainText('Strava · Sync failed');
      await expect(section).toContainText(TODAYS_RIDE);
      await expect(page.getByTestId('home-activity-row')).toHaveCount(4);
      // One time on the card: the last good sync, which is how old these rides are.
      await expect(section.getByText(/Last synced:/)).toHaveCount(1);
      await expect(section.getByText(/Last synced:/)).toContainText(SYNC_TIME.format(lastGood));
      await expectDrawnTrack(page, TODAYS_RIDE);

      const body = await (
        await ctx.get('/api/me/activities/recent', { headers: { Authorization: `Bearer ${bearer}` } })
      ).json();
      expect(Math.abs(Date.parse(body.as_of) - lastGood.getTime()), JSON.stringify(body)).toBeLessThan(2_000);
      expect(Math.abs(Date.parse(body.sync_failure.last_synced_at) - lastGood.getTime())).toBeLessThan(2_000);
      await page.screenshot({ path: test.info().outputPath('home-sync-incident.png'), fullPage: true });
    });

    await test.step("the scraper recovers with the evening's ride; the retry lands it on top with its map", async () => {
      scraper.rides = [ride('e2e-ride-4', EVENING_RIDE, 2, 1800), ...scraper.rides];
      scraper.list = 'rows';
      await failed.getByTestId('home-sync-retry').click();
      const latest = page.getByTestId('home-activity-latest');
      await expect(latest).toContainText(EVENING_RIDE, { timeout: 30_000 });
      await expect(failed).toHaveCount(0);
      await expectDrawnTrack(page, EVENING_RIDE);
    });
  });

  test('a scraper that went down is a failed sync; restarted without its sessions, the retry lands the night ride', async ({
    page,
  }) => {
    test.setTimeout(180_000);
    await hermeticBasemap(page);
    const lastGood = new Date(Date.now() - 5 * 3600_000);
    ageLastSync(userId, lastGood);
    await scraper.stop();

    await signIn(page, email, password);
    const section = page.getByTestId('home-activities');
    const failed = page.getByTestId('home-sync-failed');

    await test.step('refused connections are a failed sync, and the rides stay', async () => {
      await expect(failed).toBeVisible({ timeout: 45_000 });
      await expect(failed).toContainText('Strava · Sync failed');
      await expect(section).toContainText(EVENING_RIDE);
      await expect(section.getByText(/Last synced:/)).toContainText(SYNC_TIME.format(lastGood));
    });

    await test.step('back up with no session held: the retry imports it again and lands the night ride', async () => {
      await scraper.restart();
      scraper.rides = [ride('e2e-ride-5', NIGHT_RIDE, 1, 1800), ...scraper.rides];
      scraper.list = 'rows';
      const imports = scraper.imports();
      await failed.getByTestId('home-sync-retry').click();
      const latest = page.getByTestId('home-activity-latest');
      await expect(latest).toContainText(NIGHT_RIDE, { timeout: 30_000 });
      await expect(failed).toHaveCount(0);
      expect(scraper.imports(), 'the restarted scraper was given the session again').toBeGreaterThan(imports);
      await expectDrawnTrack(page, NIGHT_RIDE);
    });
  });

  test('a scraper shedding the read (503 scraper_busy) is a failed sync, and the retry lands once it has room', async () => {
    const auth = { Authorization: `Bearer ${bearer}` };
    const lastGood = new Date(Date.now() - 5 * 3600_000);
    ageLastSync(userId, lastGood);
    scraper.list = 'busy';
    const reads = scraper.listReads();
    const asked = Date.now();

    const first = await (await ctx.get('/api/me/activities/recent', { headers: auth })).json();
    expect(first.stale, JSON.stringify(first)).toBe(true);
    await expect
      .poll(async () => (await (await ctx.get('/api/me/activities/recent', { headers: auth })).json()).sync_failure, {
        timeout: 30_000,
      })
      .toMatchObject({ provider: 'strava' });
    expect(scraper.listReads()).toBeGreaterThan(reads);
    const failedBody = await (await ctx.get('/api/me/activities/recent', { headers: auth })).json();
    expect(failedBody.activities.map((a: { name: string }) => a.name)).toContain(NIGHT_RIDE);
    expect(Math.abs(Date.parse(failedBody.as_of) - lastGood.getTime())).toBeLessThan(2_000);
    expect(Date.parse(failedBody.sync_failure.failed_at), 'the shed read is the failure Home names').toBeGreaterThanOrEqual(
      asked - 1_000,
    );

    scraper.list = 'rows';
    const retried = await (await ctx.get('/api/me/activities/recent?retry=true', { headers: auth })).json();
    expect(retried.stale, JSON.stringify(retried)).toBe(true);
    await expect
      .poll(async () => (await (await ctx.get('/api/me/activities/recent', { headers: auth })).json()).sync_failure, {
        timeout: 30_000,
      })
      .toBeNull();
  });

  test('a detail read that never answers is answered unavailable at the route bound, and read again once the scraper answers', async () => {
    test.setTimeout(ROUTE_ANSWER_SECS * 1000 + 90_000);
    const auth = { Authorization: `Bearer ${bearer}` };
    scraper.detail = 'hangs';
    const started = Date.now();
    const route = await ctx.get('/api/me/activities/strava/e2e-ride-h/route', {
      headers: auth,
      timeout: (ROUTE_ANSWER_SECS + 30) * 1000,
    });
    const took = Date.now() - started;
    expect(await route.json()).toEqual({ route: null, reason: 'unavailable' });
    expect(took, 'answered at its bound, not held by the hung read').toBeLessThan((ROUTE_ANSWER_SECS + 5) * 1000);
    expect(took).toBeGreaterThanOrEqual((ROUTE_ANSWER_SECS - 1) * 1000);

    // The hung read gives up with the scraper's error; the retry reads again.
    scraper.release();
    scraper.detail = 'route';
    const readsBefore = scraper.detailReads();
    await expect
      .poll(
        async () =>
          (await (await ctx.get('/api/me/activities/strava/e2e-ride-h/route?retry=true', { headers: auth })).json())
            .route?.coordinates?.length ?? 0,
        { timeout: 30_000 },
      )
      .toBeGreaterThan(2);
    expect(scraper.detailReads()).toBeGreaterThan(readsBefore);
  });

  test('a route the scraper could not read answers unavailable, never "no GPS", and keeps has_gps', async () => {
    scraper.detail = 'no_route';
    const auth = { Authorization: `Bearer ${bearer}` };
    const route = await ctx.get('/api/me/activities/strava/e2e-ride-0/route', { headers: auth });
    expect(route.ok()).toBeTruthy();
    expect(await route.json()).toEqual({ route: null, reason: 'unavailable' });

    const recent = await (await ctx.get('/api/me/activities/recent?limit=20', { headers: auth })).json();
    const row = recent.activities.find((activity: { id: string }) => activity.id === 'e2e-ride-0');
    expect(row?.has_gps, JSON.stringify(recent)).toBe(true);
  });

  test('a ride the scraper read without GPS settles no_gps for good, and the list says so', async () => {
    scraper.detail = 'empty_route';
    const auth = { Authorization: `Bearer ${bearer}` };
    const readsBefore = scraper.detailReads();
    const route = await ctx.get('/api/me/activities/strava/e2e-ride-00/route', { headers: auth });
    expect(await route.json()).toEqual({ route: null, reason: 'no_gps' });
    const retried = await ctx.get('/api/me/activities/strava/e2e-ride-00/route?retry=true', { headers: auth });
    expect(await retried.json()).toEqual({ route: null, reason: 'no_gps' });
    expect(scraper.detailReads(), 'a settled answer is never read again').toBe(readsBefore + 1);

    const recent = await (await ctx.get('/api/me/activities/recent?limit=20', { headers: auth })).json();
    const row = recent.activities.find((activity: { id: string }) => activity.id === 'e2e-ride-00');
    expect(row?.has_gps, JSON.stringify(recent)).toBe(false);
  });
});
