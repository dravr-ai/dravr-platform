// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Mobile-viewport E2E for the athlete Home — lands there, Home leads the bottom bar, the page fits the phone
// ABOUTME: Tapping an activity opens chat with the draft at this width too; whatever is topmost (page, banner, admin header) clears the notch

import { test, expect, type Page } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard, setupAndLoginAsAdmin } from './test-helpers';

/**
 * What `viewport-fit=cover` plus a black-translucent status bar hands an
 * installed PWA on a notched iPhone: y=0 is under a 47px status bar.
 */
const NOTCH_INSET_PX = 47;

async function emulateNotch(page: Page) {
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('Emulation.setSafeAreaInsetsOverride', { insets: { top: NOTCH_INSET_PX } });
}

/**
 * The notch inset the first element matching `selector` takes: `pad-safe-top`
 * (index.css) is a transparent top border, added to the element's own padding.
 */
function insetOf(page: Page, selector: string) {
  return page.locator(selector).first().evaluate((el) => getComputedStyle(el).borderTopWidth);
}

/** Where the page itself begins: the top of the content area every tab renders into. */
async function pageTop(page: Page) {
  const box = await page.locator('[data-page-shell]').boundingBox();
  return box?.y ?? -1;
}

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
  // Its route was read once and the recording held no GPS.
  {
    id: 'act-3',
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
    id: 'act-2',
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
  // Garmin's activity list carries no position and this route was never
  // read: the row says it may have one, and the route endpoint answers.
  {
    id: 'act-1',
    provider: 'garmin',
    name: 'Lake loop',
    sport_type: 'run',
    start_date: '2026-09-16T11:00:00Z',
    duration_seconds: 2700,
    distance_meters: 8100,
    elevation_gain_meters: 40,
    has_gps: true,
    summary_polyline: null,
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
  title: 'Lake loop',
  source_tool: 'garmin',
};

/** One provider of `GET /api/providers`, connected; `needsReauth` is a session the athlete has to renew. */
function provider(slug: string, displayName: string, needsReauth = false) {
  return {
    provider: slug,
    display_name: displayName,
    requires_oauth: true,
    connected: true,
    needs_reauth: needsReauth,
    capabilities: ['activities'],
    consent_required: false,
  };
}

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

async function mockHome(page: Page, providers = [provider('strava', 'Strava'), provider('garmin', 'Garmin')]) {
  await page.route('**/api/providers', async (route) => {
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ providers }) });
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
  await page.route('**/api/me/activities/*/*/route', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ route: ROUTE, reason: null }),
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

  test('a latest activity whose stored route held no GPS says it has no track; tapping a row opens the analyze draft', async ({ page }) => {
    await expect(page.getByTestId('home-activity-latest')).toContainText('This activity recorded no GPS track.');
    // One sketch from the summary polyline, one from the route the endpoint
    // answers for the row whose route had never been read.
    await expect(page.getByTestId('home-activity-row')).toHaveCount(2);
    await expect(page.getByTestId('route-sketch')).toHaveCount(2);
    await expect(page.getByTestId('provider-reconnect-banner')).toHaveCount(0);

    await page.getByTestId('home-activity-row').first().getByRole('button').click();
    await expect(page).toHaveURL(/#chat\/conv-home-mobile$/);
    await expect(page.getByPlaceholder('Message Dravr...').first()).toHaveValue(/^Analyze my activity from .+ \(Run\)$/);
  });

  test('on an installed iOS PWA with no strip above, every page starts below the notch', async ({ page }) => {
    // A browser tab has no top inset: the band takes no room at all.
    expect(await insetOf(page, '[data-testid="shell-safe-top"]')).toBe('0px');
    expect(await pageTop(page)).toBe(0);

    await emulateNotch(page);
    await expect.poll(() => insetOf(page, '[data-testid="shell-safe-top"]')).toBe(`${NOTCH_INSET_PX}px`);

    // Home: its page header starts under the status bar, not behind it.
    expect(await pageTop(page)).toBeGreaterThanOrEqual(NOTCH_INSET_PX);
    const homeTitle = await page.getByTestId('home-page').getByRole('heading', { name: 'Home' }).boundingBox();
    expect(homeTitle?.y ?? 0).toBeGreaterThanOrEqual(NOTCH_INSET_PX);

    // Chat renders into the same content area, so it clears the notch too.
    await page.getByRole('navigation', { name: 'Primary navigation' }).getByRole('button', { name: 'Chat' }).click();
    await expect(page).toHaveURL(/#chat/);
    await expect(page.getByTestId('home-page')).toHaveCount(0);
    expect(await pageTop(page)).toBeGreaterThanOrEqual(NOTCH_INSET_PX);
    const chatTop = await page
      .locator('[data-page-shell] > *')
      .first()
      .evaluate((el) => el.getBoundingClientRect().top);
    expect(chatTop).toBeGreaterThanOrEqual(NOTCH_INSET_PX);

    // And Settings, whose header is its own again.
    await page.evaluate(() => {
      window.location.hash = 'settings';
    });
    await expect(page).toHaveURL(/#settings/);
    expect(await pageTop(page)).toBeGreaterThanOrEqual(NOTCH_INSET_PX);
  });
});

test.describe('Athlete Home — mobile viewport, a connection to reconnect', () => {
  test.beforeEach(async ({ page }) => {
    await setupDashboardMocks(page, { role: 'user', email: 'alice@acme.com', displayName: 'Alice Test' });
    await mockHome(page, [provider('strava', 'Strava', true), provider('garmin', 'Garmin', true)]);
    await loginToDashboard(page, { email: 'alice@acme.com', password: 'password123' });
    await expect(page.getByTestId('home-page')).toBeVisible();
  });

  test('the shell banner names the providers inside the phone width, the rows stay, and it leads to the connections pane', async ({ page }) => {
    const prompt = page.getByTestId('provider-reconnect-banner');
    await expect(prompt).toContainText('Reconnect Strava and Garmin to see your new activities.');
    await expect(page.getByTestId('home-activity-row')).toHaveCount(2);

    const viewport = page.viewportSize();
    expect(viewport).not.toBeNull();
    const box = await prompt.boundingBox();
    expect(box).not.toBeNull();
    expect(box?.x ?? -1).toBeGreaterThanOrEqual(0);
    expect((box?.x ?? 0) + (box?.width ?? 0)).toBeLessThanOrEqual((viewport?.width ?? 0) + 1);
    const { scrollWidth, clientWidth } = await page.evaluate(() => ({
      scrollWidth: document.documentElement.scrollWidth,
      clientWidth: document.documentElement.clientWidth,
    }));
    expect(scrollWidth).toBeLessThanOrEqual(clientWidth + 1);

    await prompt.getByRole('button', { name: 'Reconnect' }).click();
    await expect(page).toHaveURL(/#settings\/connections$/);
  });

  test('on an installed iOS PWA the strip pads past the notch, and gives the inset up to a strip above it', async ({ page, context }) => {
    const prompt = page.getByTestId('provider-reconnect-banner');
    await expect(prompt).toBeVisible();
    const inset = (testId: string) => insetOf(page, `[data-testid="${testId}"]`);

    // A browser tab has no top inset: the strip keeps its own layout.
    expect(await inset('provider-reconnect-banner')).toBe('0px');

    await emulateNotch(page);

    await expect.poll(() => inset('provider-reconnect-banner')).toBe(`${NOTCH_INSET_PX}px`);
    const strip = await prompt.boundingBox();
    const message = await prompt.getByText('Reconnect needed').boundingBox();
    expect(strip?.y).toBe(0);
    expect(message?.y ?? 0).toBeGreaterThanOrEqual(NOTCH_INSET_PX);
    // The page under the strip starts where the strip ends: the inset is
    // taken once, by the topmost strip, not again by what follows it.
    expect(await insetOf(page, '[data-testid="shell-safe-top"]')).toBe('0px');
    expect(await pageTop(page)).toBeCloseTo((strip?.y ?? 0) + (strip?.height ?? 0), 0);

    // An overlay is a viewport layer: the reconnect strip does not cover it,
    // so the info panel takes the inset itself even with the strip above.
    // The thread's info panel reads the conversation from the list, so the
    // list answers with it (a later route takes precedence over mockHome's).
    await page.route(/\/api\/chat\/conversations(\?.*)?$/, async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ conversations: [CONVERSATION], total: 1, limit: 50, offset: 0 }),
      });
    });
    await page.evaluate(() => {
      window.location.hash = 'chat/conv-home-mobile';
    });
    await page.getByTestId('conversation-header-title').click();
    const panel = page.getByTestId('conversation-info-panel');
    await expect(panel).toBeVisible();
    await expect.poll(() => insetOf(page, '[data-testid="conversation-info-panel"]')).toBe(`${NOTCH_INSET_PX}px`);
    const panelHeading = await panel.getByRole('heading').first().boundingBox();
    expect(panelHeading?.y ?? 0).toBeGreaterThanOrEqual(NOTCH_INSET_PX);
    await page.keyboard.press('Escape');
    await expect(panel).toHaveCount(0);

    // The offline strip mounts above the whole shell. It is now the topmost,
    // so it takes the inset and the reconnect strip under it gives it up.
    await context.setOffline(true);
    await expect(page.getByTestId('offline-banner')).toBeVisible();
    await expect.poll(() => inset('offline-banner')).toBe(`${NOTCH_INSET_PX}px`);
    await expect.poll(() => inset('provider-reconnect-banner')).toBe('0px');
    await context.setOffline(false);
  });
});

test.describe('Operator shell — mobile viewport', () => {
  test('on an installed iOS PWA the admin header is topmost, so it takes the inset and the band under it does not', async ({ page }) => {
    await setupAndLoginAsAdmin(page);
    const header = page.locator('main > header');
    await expect(header).toBeVisible();

    await emulateNotch(page);
    await expect.poll(() => insetOf(page, 'main > header')).toBe(`${NOTCH_INSET_PX}px`);
    const title = await header.getByRole('heading', { level: 1 }).boundingBox();
    expect(title?.y ?? 0).toBeGreaterThanOrEqual(NOTCH_INSET_PX);
    expect(await insetOf(page, '[data-testid="shell-safe-top"]')).toBe('0px');
    const box = await header.boundingBox();
    expect(await pageTop(page)).toBeCloseTo((box?.y ?? 0) + (box?.height ?? 0), 0);
  });
});

test.describe('Signed-out screens — mobile viewport', () => {
  test('on an installed iOS PWA the sign-in page starts below the notch, once, even under the offline strip', async ({ page, context }) => {
    await setupDashboardMocks(page, { role: 'user', email: 'alice@acme.com', displayName: 'Alice Test' });
    await page.goto('/');
    const title = page.getByRole('heading', { level: 1 });
    await expect(title).toBeVisible();
    const screenRoot = 'div.min-h-dvh.pad-safe-top';
    expect(await insetOf(page, screenRoot)).toBe('0px');

    await emulateNotch(page);
    await expect.poll(() => insetOf(page, screenRoot)).toBe(`${NOTCH_INSET_PX}px`);
    const box = await title.boundingBox();
    expect(box?.y ?? 0).toBeGreaterThanOrEqual(NOTCH_INSET_PX);
    // The screen still fits the viewport: the inset sits inside min-h-dvh.
    const { scrollHeight, innerHeight } = await page.evaluate(() => ({
      scrollHeight: document.documentElement.scrollHeight,
      innerHeight: window.innerHeight,
    }));
    expect(scrollHeight).toBeLessThanOrEqual(innerHeight + 1);

    // The offline strip mounts above the sign-in screen: it takes the inset
    // and the screen under it does not take it a second time.
    await context.setOffline(true);
    await expect(page.getByTestId('offline-banner')).toBeVisible();
    await expect.poll(() => insetOf(page, '[data-testid="offline-banner"]')).toBe(`${NOTCH_INSET_PX}px`);
    await expect.poll(() => insetOf(page, screenRoot)).toBe('0px');
    await context.setOffline(false);
  });
});
