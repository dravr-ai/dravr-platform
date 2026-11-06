// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Text painted over text: the coach roster, the shared tabs and Select, the admin rows, the onboarding stepper, in French with long values
// ABOUTME: Every route is mocked, and each view states how much text it must have measured so an empty page cannot pass

import { test, expect, type Page } from '@playwright/test';
import type { CoachRosterResponse } from '@pierre/shared-types';
import { setupDashboardMocks, loginToDashboard, mockAdminUserDrawer, signInThroughHostedPage } from './test-helpers';
import { mockCoach, localToday } from './coach-home-mocks';
import { expectNoCollidingText, useFrench } from './text-collision';

const NOW = '2026-11-01T12:00:00Z';
const json = (body: unknown) => ({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });
const envelope = (data: unknown) => json({ success: true, message: 'ok', data });

const LONG_NAME = 'Marie-Ève Bélanger-Desrochers de la Chevrotière';
const LONG_RACE = 'Championnat canadien Ironman 70.3 de Mont-Tremblant';

async function go(page: Page, hash: string) {
  await page.evaluate((target) => {
    window.location.hash = target;
  }, hash);
}

/** The roster with the values that overran: long names, six sources, the build phase, every agent state. */
function longRoster(roster: CoachRosterResponse): CoachRosterResponse {
  const today = localToday();
  const source = roster.athletes.find((athlete) => athlete.user_id === 'u-lea');
  if (source === undefined) throw new Error('the fixture lost u-lea');
  const clone = (id: string, name: string) => {
    const athlete = structuredClone(source);
    athlete.user_id = id;
    athlete.name = name;
    if (athlete.plan !== undefined) {
      athlete.plan.phase = 'build';
      athlete.plan.week = 10;
      if (athlete.plan.goal_race) athlete.plan.goal_race.name = LONG_RACE;
    }
    if (athlete.season?.state === 'a_race_ahead' && athlete.season.race) athlete.season.race.name = LONG_RACE;
    if (athlete.training) {
      athlete.training.last_activity_per_source = { garmin: today, strava: today, whoop: today, intervals: today, wahoo: today, trainingpeaks: today };
      athlete.training.needs_reconnect = ['strava', 'whoop'];
    }
    return athlete;
  };
  const partial = clone('u-long-1', LONG_NAME);
  partial.agent_replies = { count: 12, verified: 7, unmatched_figures: 0, capped: false };
  const unverified = clone('u-long-2', 'Jean-Christophe Vaillancourt-Létourneau');
  unverified.agent_replies = { count: 3, verified: 0, unmatched_figures: 3, capped: false };
  const none = clone('u-long-3', 'Anne-Sophie Charbonneau-Thibodeau');
  none.agent_replies = { count: 0, verified: 0, unmatched_figures: 0, capped: false };
  if (none.training) none.training.form = null;
  return { ...roster, athletes: [partial, unverified, none, ...roster.athletes] };
}

test.describe('Text never paints over text', () => {
  test.beforeEach(async ({ page }) => {
    await useFrench(page);
  });

  for (const [width, height] of [[1440, 900], [1280, 800], [768, 1024]] as const) {
    test(`the coach roster at ${width}`, async ({ page }) => {
      await page.setViewportSize({ width, height });
      await mockCoach(page, { language: 'fr', roster: longRoster });
      await go(page, 'home/team');
      await expect(page.getByTestId('coach-row-u-long-1')).toBeVisible();
      // A row with no agent reply: its French words all stay, in their own column.
      await expect(page.getByTestId('coach-row-u-long-3')).toContainText('Aucune cette semaine');
      await expectNoCollidingText(page, `roster ${width}`, '[data-testid="coach-roster"]', 100, '[data-testid="coach-roster-scroll"]');
    });
  }

  test('a capped source cell still names every source and shows the broken ones first', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await mockCoach(page, { language: 'fr', roster: longRoster });
    await go(page, 'home/team');
    const row = page.getByTestId('coach-row-u-long-1');
    await expect(row).toBeVisible();
    const chips = row.getByRole('cell').nth(3).getByRole('img');
    await expect(chips).toHaveCount(3);
    await expect(chips.nth(2)).toHaveText('+4');
    await expect(chips.nth(2)).toHaveAttribute('aria-label', /Garmin/);
    await expect(chips.first()).toHaveAttribute('aria-label', /Strava|WHOOP|Whoop/);
  });

  for (const [width, height] of [[1280, 800], [768, 1024]] as const) {
    test(`the season at ${width}, on both scales`, async ({ page }) => {
      await page.setViewportSize({ width, height });
      await mockCoach(page, { language: 'fr', roster: longRoster });
      await go(page, 'home/season');
      await expect(page.getByTestId('coach-season-today')).toBeVisible();
      await expectNoCollidingText(page, `season ${width}`, '[data-testid="coach-home"]', 30);

      // Measured against the row rather than today's date: the pill lands on a month name only some days.
      const pill = await page.getByTestId('coach-season-today').boundingBox();
      const month = await page.getByRole('columnheader').nth(1).locator('span[aria-hidden="true"]').first().evaluate((el) => {
        const range = document.createRange();
        range.selectNodeContents(el);
        return range.getBoundingClientRect().top;
      });
      expect(pill).not.toBeNull();
      expect((pill?.y ?? 0) + (pill?.height ?? 0)).toBeLessThanOrEqual(month);

      await page.getByTestId('coach-season-scale-12w').click();
      await expect(page.getByTestId('coach-season-scale-12w')).toHaveAttribute('aria-pressed', 'true');
      await expectNoCollidingText(page, `season 12w ${width}`, '[data-testid="coach-home"]', 30);
    });
  }

  // 1440 too: on a desktop the sheet is still 360px wide, so the row's layout cannot follow the viewport.
  for (const width of [390, 768, 1440]) {
    test(`the notifications tabs and rows at ${width}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await setupDashboardMocks(page, { role: 'user', language: 'fr' });
      await page.route('**/api/notifications**', (route) =>
        route.fulfill(
          json(
            route.request().url().includes('unread-count')
              ? { count: 0 }
              : {
                  data: [
                    {
                      id: 'n-1',
                      category: 'achievement',
                      notification_type: 'fitness_improvement',
                      title: 'Nouveau message',
                      body: 'Discussion Sep 22 18:38 t’a envoyé un message',
                      data: null,
                      image_url: null,
                      read_at: NOW,
                      delivered_at: null,
                      opened_at: null,
                      created_at: NOW,
                    },
                  ],
                  total: 1,
                  unread_count: 0,
                },
          ),
        ),
      );
      await loginToDashboard(page);
      await go(page, 'discover');
      await page.getByTestId('notification-bell').click();
      await expect(page.getByTestId('notification-row-n-1')).toBeVisible({ timeout: 15000 });
      await expectNoCollidingText(page, `notifications ${width}`, '[data-testid="notification-sheet"]', 10);
      // A word wider than its column would be cut mid-letter by the clamp.
      const cut = await page.getByTestId('notification-row-n-1').locator('p').evaluateAll((all) =>
        all.filter((p) => p.scrollWidth > p.clientWidth + 1).map((p) => p.textContent),
      );
      expect(cut).toEqual([]);
      // Beside the category word the message is left a sliver it can barely read in.
      const message = await page.getByTestId('notification-row-n-1').locator('p').last().boundingBox();
      expect(message?.width ?? 0).toBeGreaterThanOrEqual(150);
    });
  }

  test('the memory filter tabs on a phone', async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await setupDashboardMocks(page, { role: 'user', language: 'fr' });
    await page.route('**/api/memory/facts**', (route) => route.fulfill(json({ facts: [], total: 0 })));
    await loginToDashboard(page);
    await go(page, 'settings/memory');
    const filter = page.getByTestId('memory-kind-filter');
    await expect(filter).toBeVisible({ timeout: 15000 });
    // The row scrolls, so how many words one reading sees depends on the font: count
    // the tabs, and read both ends of the row so none goes unmeasured.
    await expect.poll(() => filter.getByRole('button').count()).toBeGreaterThan(5);
    await expectNoCollidingText(page, 'memory filter', '[data-testid="memory-kind-filter"]', 2, '[data-testid="memory-kind-filter"]');
    await filter.evaluate((el) => {
      el.scrollLeft = 0;
    });
    // Centred in a box narrower than itself, a word starts left of a scroller that cannot reach it.
    const start = await filter.evaluate((el) => {
      const range = document.createRange();
      range.selectNodeContents(el.querySelector('button') as HTMLElement);
      return range.getBoundingClientRect().left - el.getBoundingClientRect().left;
    });
    expect(start).toBeGreaterThanOrEqual(0);
  });

  test('a Select keeps its words clear of the chevron', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await setupDashboardMocks(page, { role: 'user', language: 'fr' });
    await page.route('**/api/me/check-in', (route) => route.fulfill(json({ cadence: 'off' })));
    await loginToDashboard(page);
    await go(page, 'settings/profile');
    const select = page.getByTestId('check-in-settings-select');
    await expect(select).toBeVisible({ timeout: 15000 });
    const measured = await select.evaluate((el) => {
      const field = el as HTMLSelectElement;
      const style = getComputedStyle(field);
      const context = document.createElement('canvas').getContext('2d') as CanvasRenderingContext2D;
      context.font = style.font;
      const gutter = parseFloat(style.paddingRight);
      return {
        gutter,
        ellipsis: style.textOverflow,
        room: field.clientWidth - gutter,
        words: Math.max(...Array.from(field.options).map((option) => context.measureText(option.text).width)),
      };
    });
    expect(measured.gutter).toBeGreaterThanOrEqual(24);
    expect(measured.ellipsis).toBe('ellipsis');
    // The cadence most athletes read: its longest sentence fits whole.
    expect(measured.words).toBeLessThanOrEqual(measured.room);
  });

  for (const width of [1440, 390]) {
    test(`the user drawer's usage rows stay on their lines at ${width}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await setupDashboardMocks(page, { role: 'super_admin', language: 'fr' });
      await mockAdminUserDrawer(page, NOW);
      await loginToDashboard(page);
      await go(page, 'users');
      await page.getByRole('button', { name: /^(Active|Actifs?)/ }).first().click();
      await page.getByText('alice@acme.com').first().click();
      const name = page.getByText('anthropic/claude-sonnet-5-5');
      await expect(name).toHaveCount(4, { timeout: 15000 });
      // Overlap is not the only scramble: a name or figure broken across lines
      // overlaps nothing and still reads as noise.
      const broken = await page.evaluate(() => {
        const lines = (el: Element) => {
          const range = document.createRange();
          range.selectNodeContents(el);
          return new Set(Array.from(range.getClientRects()).filter((r) => r.width > 1).map((r) => Math.round(r.top))).size;
        };
        const words = /^(anthropic\/claude-sonnet-5-5|[\d\s,.\u00a0\u202f]+ tok)$/;
        return Array.from(document.querySelectorAll('span, div'))
          .filter((el) => el.children.length === 0 && words.test((el.textContent ?? '').trim()) && lines(el) > 1)
          .map((el) => el.textContent);
      });
      expect(broken, 'a usage row broke across lines').toEqual([]);
      await expect(page.getByText('memory_extraction', { exact: true })).toBeVisible();
    });
  }

  test('the admin activity rows at 1024', async ({ page }) => {
    await page.setViewportSize({ width: 1024, height: 768 });
    await setupDashboardMocks(page, { role: 'super_admin', language: 'fr' });
    await page.route('**/api/admin/analytics/recent-activity', (route) =>
      route.fulfill(
        json({
          recent_llm_calls: ['language_classification', 'compaction_summary'].map((call_type, index) => ({
            id: `l${index}`,
            provider: 'claude_code',
            model: 'claude-sonnet',
            total_tokens: 128400,
            cached_tokens: 88000,
            cached_write_tokens: 12000,
            cached_write_1h_tokens: 4000,
            cost_usd: 1.2345,
            call_type,
            execution_time_ms: 128400,
            created_at: NOW,
          })),
          recent_conversations: [],
          summary: { active_conversations: 1, llm_calls_today: 2, total_tokens_today: 256800, estimated_cost_today: 2.5 },
        }),
      ),
    );
    await loginToDashboard(page);
    await go(page, 'activity');
    await expect(page.getByText('language_classification').first()).toBeVisible({ timeout: 15000 });
    await expectNoCollidingText(page, 'activity 1024', 'main', 15);
  });

  test('the sign-up funnel stays inside its tiles at 1024', async ({ page }) => {
    await page.setViewportSize({ width: 1024, height: 768 });
    await setupDashboardMocks(page, { role: 'super_admin', language: 'fr' });
    await page.route('**/api/admin/analytics/new-users**', (route) =>
      route.fulfill(
        envelope({
          window_days: 30,
          generated_at: NOW,
          truncated: false,
          funnel: { signed_up: 12840, approved_or_active: 11230, email_verified: 10950, provider_connected: 8421, first_sync: 8120, first_turn: 7345, day_7_eligible: 6120, active_after_day_7: 4210 },
          users: [],
        }),
      ),
    );
    await loginToDashboard(page);
    await go(page, 'users');
    await page.getByRole('button', { name: /Nouveaux comptes/ }).click();
    const funnel = page.getByTestId('new-users-funnel');
    await expect(funnel).toBeVisible({ timeout: 15000 });
    const escaped = await funnel.locator(':scope > div').evaluateAll((tiles) =>
      tiles.flatMap((tile) => {
        const edge = tile.getBoundingClientRect().right;
        return Array.from(tile.querySelectorAll('p')).filter((p) => {
          const range = document.createRange();
          range.selectNodeContents(p);
          return Array.from(range.getClientRects()).some((rect) => rect.right > edge);
        }).map((p) => p.textContent);
      }),
    );
    expect(escaped).toEqual([]);
  });

  test('a feature flag key never runs under its buttons', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await setupDashboardMocks(page, { role: 'super_admin', language: 'fr' });
    await page.route('**/api/admin/users**', (route) =>
      new URL(route.request().url()).pathname !== '/api/admin/users' ? route.fallback() : route.fulfill(
        envelope({
          users: [{ id: 'user-1', user_id: 'user-1', email: 'marie@test.com', display_name: LONG_NAME, is_admin: false, role: 'user', status: 'active', user_status: 'active', tier: 'starter', created_at: NOW, last_active: NOW, last_active_at: NOW, approved_at: NOW, tenant_id: 'tenant-a' }],
          total: 1,
          has_more: false,
        }),
      ),
    );
    // The drawer reads all of these, and an unmocked one crashes it before the flags paint.
    await page.route('**/api/admin/users/*/rate-limit', (route) =>
      route.fulfill(envelope({ user_id: 'user-1', tier: 'starter', rate_limits: { monthly: { limit: 1000, used: 10, remaining: 990 } }, reset_times: { monthly_reset: NOW }, override_active: false, override_note: null })),
    );
    await page.route('**/api/admin/users/*/activity**', (route) =>
      route.fulfill(envelope({ user_id: 'user-1', period_days: 30, total_requests: 0, top_tools: [] })),
    );
    await page.route('**/api/admin/users/*/usage**', (route) =>
      route.fulfill(json({ user_id: 'user-1', from: NOW, total_cost_usd: 0, by_model: [], daily: [] })),
    );
    await page.route('**/api/admin/users/*/admin-profile', (route) =>
      route.fulfill(json({ user_id: 'user-1', coaching_persona: 'balanced', default_coach_id: null, manages_roster: false, manages_roster_operator_grant: null, installed_agents: [], joined_groups: [] })),
    );
    await page.route('**/api/admin/users/*/features', (route) =>
      route.fulfill(
        json({
          rows: [],
          known: [{ key: 'persona_notification_policy', description: 'Which notifications a persona may send.', default_enabled: true }],
        }),
      ),
    );
    await loginToDashboard(page);
    await go(page, 'users');
    await page.getByRole('button', { name: /All Users/ }).click();
    await page.getByText(LONG_NAME).first().click();
    await expect(page.getByText('persona_notification_policy')).toBeVisible({ timeout: 15000 });
    const row = page.getByText('persona_notification_policy').locator('xpath=ancestor::div[contains(@class,"justify-between")][1]');
    await expect(row).toBeVisible();
    const clash = await row.evaluate((el) => {
      const words = Array.from(el.querySelectorAll('code, span')).map((node) => {
        const range = document.createRange();
        range.selectNodeContents(node);
        return range.getBoundingClientRect().right;
      });
      const button = el.querySelector('button')?.getBoundingClientRect().left ?? Infinity;
      return Math.max(...words) - button;
    });
    expect(clash).toBeLessThanOrEqual(0);
  });

  for (const width of [390, 768]) {
    test(`the onboarding stepper fits at ${width}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await setupDashboardMocks(page, { role: 'user', email: 'fresh@test.com', displayName: 'Fresh User', language: 'fr' });
      await page.route('**/api/me/onboarding-status', (route) => route.fulfill(json({ needs_provider_connection: true })));
      await page.goto('/');
      await signInThroughHostedPage(page);
      const stepper = page.locator('nav').filter({ has: page.locator('ol') }).first();
      await expect(stepper).toBeVisible({ timeout: 15000 });
      const measured = await stepper.evaluate((nav) => {
        const edges = Array.from(nav.querySelectorAll('ol span, ol div, p')).filter((el) => {
          const box = el.getBoundingClientRect();
          return box.width > 0 && getComputedStyle(el).display !== 'none';
        }).map((el) => el.getBoundingClientRect());
        return {
          left: Math.min(...edges.map((box) => box.left)),
          right: Math.max(...edges.map((box) => box.right)),
          view: document.documentElement.clientWidth,
          sideways: document.documentElement.scrollWidth - document.documentElement.clientWidth,
        };
      });
      expect(measured.left).toBeGreaterThanOrEqual(0);
      expect(measured.right).toBeLessThanOrEqual(measured.view);
      expect(measured.sideways).toBeLessThanOrEqual(0);
      // The dots alone would say nothing: the step the athlete is on keeps its name.
      await expect(stepper.getByTestId('onboarding-current-step')).toBeVisible();
    });
  }

  test('a connected provider card awaiting activities holds together at 375', async ({ page }) => {
    await page.setViewportSize({ width: 375, height: 800 });
    await setupDashboardMocks(page, { role: 'user', language: 'fr', skipProvidersRoute: true });
    await page.route('**/api/providers', (route) =>
      route.fulfill(
        json({
          providers: [
            { provider: 'sciotte', display_name: 'Strava', description: 'Activités', connected: true, requires_oauth: true, capabilities: [], oauth_awaiting_activities: true },
          ],
        }),
      ),
    );
    await page.route('**/api/me/onboarding-status', (route) =>
      route.fulfill(
        json({
          needs_provider_connection: true,
          // Past the steps before this one, which is where the connected card shows.
          steps: ['profile_type', 'coach_proposal', 'about_you', 'parq', 'check_in'].map((step_id) => ({ step_id, status: 'complete' })),
        }),
      ),
    );
    await page.goto('/');
    await signInThroughHostedPage(page);
    const row = page.getByTestId('provider-glyph-sciotte').locator('xpath=ancestor::button[1]');
    await expect(row).toBeVisible({ timeout: 15000 });
    await expect(row).toContainText('Strava');
    await expect(row).toContainText('Importer mon historique');
    await expectNoCollidingText(page, 'provider row', '[data-testid="onboarding-shell"]', 5);
    const past = await row.evaluate((button) => {
      const edge = button.getBoundingClientRect().right;
      return Math.max(...Array.from(button.querySelectorAll('span')).map((span) => span.getBoundingClientRect().right)) - edge;
    });
    expect(past).toBeLessThanOrEqual(0);
  });

  test('the Discover header on a phone', async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await setupDashboardMocks(page, { role: 'user', language: 'fr' });
    await loginToDashboard(page);
    await go(page, 'discover');
    const title = page.getByRole('heading', { level: 2, name: 'Découvrir' });
    await expect(title).toBeVisible({ timeout: 15000 });
    const measured = await title.evaluate((heading) => {
      const search = document.querySelector('input[type="search"]') as HTMLInputElement;
      const range = document.createRange();
      range.selectNodeContents(heading);
      return {
        titleRight: range.getBoundingClientRect().right,
        searchLeft: (search.parentElement as HTMLElement).getBoundingClientRect().left,
        placeholder: getComputedStyle(search, '::placeholder').textOverflow,
      };
    });
    expect(measured.titleRight).toBeLessThanOrEqual(measured.searchLeft);
    expect(measured.placeholder).toBe('ellipsis');
  });
});
