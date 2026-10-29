// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Shared test helper functions for Playwright E2E tests.
// ABOUTME: Provides reusable authentication mocks and login helpers.

import fs from 'fs';
import path from 'path';
import type { Page, Route } from '@playwright/test';

interface UserOptions {
  role?: 'user' | 'admin' | 'super_admin';
  email?: string;
  displayName?: string;
  status?: 'active' | 'pending' | 'suspended';
  /**
   * Skip the default `/api/providers` mock below, leaving the spec's own the
   * only handler for that route.
   *
   * A spec cannot achieve this by registering its own route instead. Playwright
   * matches handlers newest-first, so one registered before `loginAsUser` loses
   * to the default; and one registered after it wins too late, because the
   * providers query has already been answered and cached under
   * `QUERY_KEYS.providers.status()` by then — leaving the screen under test
   * showing the default payload no matter what the spec mocked. Opting out is
   * what lets a spec own the answer from the very first request, including
   * while it is still in flight.
   */
  skipProvidersRoute?: boolean;
  /**
   * The athlete's language, which the server words its own content in — the
   * empty thread's starters (`GET /api/chat/starters`). English when absent;
   * a spec that switches the app to another language passes it here too, so
   * the mocked server agrees with the chrome it is swept against.
   */
  language?: string;
}

/**
 * The onboarding status of a fully onboarded user: a provider connected and the
 * agent step answered. The agent step is derived from server state (carnet#1042),
 * so a status with no step record would route every spec to the proposal.
 */
export const ONBOARDED_STATUS = {
  needs_provider_connection: false,
  steps: [{ step_id: 'coach_proposal', status: 'complete' }],
};

/**
 * Stubs that EVERY E2E test needs but many spec-local login helpers
 * never added — pin the theme to light + return sane defaults for the
 * feature-flag endpoints introduced in e25417e6. Call from any spec-local
 * `loginAsX` helper to avoid having the FeatureFlagsPanel fetch break
 * downstream assertions.
 */
export async function applyTestStubs(page: Page) {
  await page.addInitScript(() => {
    try {
      // Pin theme=light only if a spec hasn't already chosen one.
      // theme.spec.ts intentionally sets dark to test that path.
      if (window.localStorage.getItem('dravr.theme') === null) {
        window.localStorage.setItem('dravr.theme', 'light');
      }
    } catch { /* */ }
  });
  await page.route('**/api/me/features', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        flags: { api_tokens: true, billing_header: true },
        known: [
          { key: 'api_tokens', description: 'API Tokens tab', default_enabled: false },
          { key: 'billing_header', description: 'Billing header card', default_enabled: false },
        ],
      }),
    });
  });
  await page.route('**/api/admin/users/*/features', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ rows: [], known: [] }),
    });
  });
  await page.route('**/api/admin/tenants/*/features', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ rows: [], known: [] }),
    });
  });
  // The athlete Home page, where a regular user lands after sign-in — so
  // every spec that signs one in reaches it, including the spec-local login
  // helpers that skip `setupDashboardMocks`. The defaults are an athlete with
  // no plan, no training status and an empty activity cache: the page's honest empty states, never
  // invented rows. `home.spec.ts` registers its own answers after these, which
  // win.
  await page.route('**/api/me/training-plan**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ plan: null, today: new Date().toISOString().slice(0, 10) }),
    });
  });
  await page.route('**/api/me/training-status', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        today: new Date().toISOString().slice(0, 10),
        form: null,
        trend: [],
        load_ratio: null,
        recovery_days: null,
      }),
    });
  });
  await page.route('**/api/me/training-volume', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ today: new Date().toISOString().slice(0, 10), weeks: [] }),
    });
  });
  // Home's stored choices (carnet#820, carnet#1098): the plan suggestion and
  // Getting started are shown; a PUT merges the choices it names, as the
  // server does, and answers with every choice as it now stands.
  let homePreferences = { plan_suggestion_hidden: false, getting_started_hidden: false };
  await page.route('**/api/me/home-preferences', async (route) => {
    if (route.request().method() === 'PUT') {
      homePreferences = { ...homePreferences, ...(route.request().postDataJSON() as Partial<typeof homePreferences>) };
    }
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(homePreferences),
    });
  });
  // Getting started (carnet#1098): the mocked athlete is an established one,
  // every first step done over a week ago, so the section is gone. A spec of
  // a first arrival overrides this with `mockHomeFirstArrival`.
  await page.route('**/api/me/getting-started', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(GETTING_STARTED_DONE),
    });
  });
  // The athlete's units (carnet#835, carnet#1044): metric until a PUT
  // chooses otherwise; a PUT answers with what it stored.
  await page.route('**/api/me/units**', async (route) => {
    const request = route.request();
    const update = request.method() === 'PUT' ? (request.postDataJSON() as { units: string }) : null;
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ units: update?.units === 'imperial' ? 'imperial' : 'metric' }),
    });
  });
  await page.route('**/api/me/activities/recent**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ activities: [], as_of: null, stale: false }),
    });
  });
  await page.route('**/api/me/calendar**', (route) =>
    fulfillCalendar(route, { today: new Date().toISOString().slice(0, 10), plan: null }),
  );
  await page.route('**/api/me/activities/*/*/route', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ route: null, reason: 'no_gps' }),
    });
  });
  // The gear ledger (carnet#711): an athlete who declared no pair, so an
  // activity view's shoe picker and Settings › Shoes show their empty states.
  await page.route('**/api/me/gear', async (route) => {
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ gear: [] }) });
  });
  await page.route('**/api/me/activities/*/*/gear', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ gear_id: null, source: null }),
    });
  });

  // Default onboarding status for spec-local login helpers that skip
  // `setupDashboardMocks`. Specs exercising the forced-onboarding flow
  // override with `needs_provider_connection: true` before calling login.
  await page.route('**/api/me/onboarding-status', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(ONBOARDED_STATUS),
    });
  });
}

/** What a mocked `GET /api/me/calendar` answers from. */
export interface CalendarMock {
  today: string;
  /** The plan card the spec mocks (anything with `weeks`), or null for no plan. */
  plan: unknown;
  /** Home rows the spec mocks; each is placed on the UTC day of its start. */
  activities?: unknown;
  /** The first day the mocked cache holds in full; 180 days before today when omitted. */
  historyStart?: string;
}

function civilDaysAfter(date: string, days: number): string {
  const ms = Date.parse(`${date}T00:00:00Z`) + days * 86_400_000;
  return new Date(ms).toISOString().slice(0, 10);
}

function startedRows(value: unknown): Array<{ start_date: string }> {
  return Array.isArray(value)
    ? value.filter((row): row is { start_date: string } => typeof row?.start_date === 'string')
    : [];
}

function planWeeksOf(plan: unknown): Array<{ week_start: string }> | null {
  if (typeof plan !== 'object' || plan === null || !Array.isArray((plan as { weeks?: unknown }).weeks)) return null;
  return (plan as { weeks: unknown[] }).weeks.filter(
    (week): week is { week_start: string } => typeof (week as { week_start?: unknown })?.week_start === 'string',
  );
}

/**
 * Answer a calendar read the way the server does from the spec's own plan
 * and activity mocks: the activities on the asked days, oldest first, and the
 * plan weeks overlapping them — so the week strip and the month sheet show
 * what the spec's Today and activity list show. The fixtures start their
 * activities away from midnight, so the UTC day is the athlete's day.
 */
export async function fulfillCalendar(route: Route, mock: CalendarMock) {
  const url = new URL(route.request().url());
  const from = url.searchParams.get('from') ?? mock.today;
  const to = url.searchParams.get('to') ?? mock.today;
  const historyStart = mock.historyStart ?? civilDaysAfter(mock.today, -180);
  const activities = startedRows(mock.activities)
    .map((activity) => ({ date: activity.start_date.slice(0, 10), activity }))
    .filter((entry) => entry.date >= from && entry.date <= to && entry.date >= historyStart)
    .sort((a, b) => a.activity.start_date.localeCompare(b.activity.start_date));
  const planWeeks =
    planWeeksOf(mock.plan)?.filter((week) => week.week_start <= to && civilDaysAfter(week.week_start, 6) >= from) ??
    null;
  await route.fulfill({
    status: 200,
    contentType: 'application/json',
    body: JSON.stringify({ today: mock.today, from, to, history_start: historyStart, activities, plan_weeks: planWeeks }),
  });
}

/** The use cases every mocked dashboard is offered, in rank order. */
const MOCK_STARTER_IDS = ['connect', 'about_me_more', 'can_see'];

/**
 * The starters `GET /api/chat/starters` answers, worded in `language` as the
 * server words them: each label is the use case's own in the shared corpus.
 * Read from disk rather than imported, as the locale sweep reads it, since
 * Playwright's loader refuses the package's JSON imports.
 */
function mockStarters(language = 'en'): { id: string; label: string; postback: string }[] {
  const corpus = JSON.parse(
    fs.readFileSync(
      path.resolve(process.cwd(), '../packages/i18n/src/locales', language, 'translation.json'),
      'utf-8',
    ),
  ) as { use_cases: Record<string, { label: string }> };
  return MOCK_STARTER_IDS.map((id, slot) => ({
    id,
    label: corpus.use_cases[id].label,
    postback: `uc:${slot}:${id}`,
  }));
}

/** The starters an English-speaking mocked dashboard is offered. */
export const MOCK_STARTERS = mockStarters('en');

/** The thread a mocked server forges for an athlete with none of their own. */
export const MOCK_FORGED_PERSONAL_ID = 'conv-personal';

interface MockedListRow {
  id: string;
  group_id?: string | null;
  personal?: boolean;
  updated_at?: string;
  last_message?: { created_at?: string } | null;
}

/** The thread a mocked server forges when "+" finds the athlete has none. */
function mockForgedPersonalThread(): Record<string, unknown> {
  return {
    id: MOCK_FORGED_PERSONAL_ID,
    title: 'Dravr',
    agent_id: null,
    group_id: null,
    personal: true,
    message_count: 0,
    total_tokens: 0,
    created_at: '2026-10-01T09:00:00Z',
    updated_at: '2026-10-01T09:00:00Z',
  };
}

/**
 * The athlete's personal thread as a mocked server finds it (carnet#840): the
 * row flagged `personal`, else the newest row with no group, else `null`. The
 * list is read with the page's own fetch, which the page's routes answer, so
 * whichever list mock a spec registered is the one adopted from; a page that
 * cannot fetch (mid-navigation) finds none.
 */
async function mockPersonalThread(page: Page): Promise<Record<string, unknown> | null> {
  let rows: MockedListRow[];
  try {
    rows = await page.evaluate(async () => {
      const response = await fetch('/api/chat/conversations?limit=50&offset=0');
      if (!response.ok) return [];
      const body = (await response.json()) as { conversations?: MockedListRow[] };
      return body.conversations ?? [];
    });
  } catch {
    return null;
  }
  const activity = (row: MockedListRow) => row.last_message?.created_at ?? row.updated_at ?? '';
  const own = rows
    .filter((row) => !row.group_id)
    .sort((a, b) => activity(b).localeCompare(activity(a)));
  const chosen = rows.find((row) => row.personal) ?? own[0];
  return chosen ? { ...chosen, personal: true } : null;
}

/**
 * Sets up common API mocks for authenticated dashboard access.
 * This must be called BEFORE navigating to any page.
 */
export async function setupDashboardMocks(page: Page, userOptions: UserOptions = {}) {
  const {
    role = 'admin',
    email = 'admin@test.com',
    displayName = 'Test Admin',
    status = 'active',
    skipProvidersRoute = false,
    language = 'en',
  } = userOptions;

  // Theme pin + feature-flag stubs shared with spec-local helpers.
  await applyTestStubs(page);

  // Mock setup status
  await page.route('**/admin/setup/status', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ needs_setup: false, admin_user_exists: true }),
    });
  });

  // Mock the first-party token endpoint: the hosted sign-in's code exchange
  // (grant_type=authorization_code), which answers what the password grant did.
  await page.route('**/oauth/token', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        access_token: 'test-jwt-token',
        token_type: 'Bearer',
        expires_in: 86400,
        csrf_token: 'test-csrf-token',
        user: {
          id: 'user-123',
          user_id: 'user-123',
          email,
          display_name: displayName,
          role,
          is_admin: role === 'admin' || role === 'super_admin',
          user_status: status,
          tier: role === 'super_admin' ? 'enterprise' : 'professional',
          tenant_id: 'user-123',
        },
      }),
    });
  });

  // Mock analytics
  await page.route('**/api/dashboard/analytics**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ daily_usage: [] }),
    });
  });

  // Mock pending users
  await page.route('**/api/admin/pending-users', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ success: true, message: 'Retrieved 0 pending users', data: { count: 0, users: [] } }),
    });
  });

  // Mock admin users list
  await page.route('**/api/admin/users**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ success: true, message: 'Retrieved users', data: { users: [], total: 0, has_more: false } }),
    });
  });

  // Mock user stats endpoint (used by UserHome component for non-admin users)
  await page.route('**/api/user/stats', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        connected_providers: 1,
        activities_synced: 42,
        days_active: 7,
      }),
    });
  });

  // Mock MCP tokens endpoint (used by MCPTokensTab for non-admin users)
  await page.route('**/api/user/mcp-tokens', async (route, request) => {
    if (request.method() === 'GET') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ tokens: [] }),
      });
    } else {
      await route.fallback();
    }
  });

  // Mock chat conversations endpoint. Rows carry the unified-list shape —
  // coach handle/title, group id/name, last_message and unread_count — so a
  // spec that overrides this with its own rows starts from the real contract.
  await page.route('**/api/chat/conversations**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ conversations: [], total: 0, limit: 50, offset: 0 }),
    });
  });

  // The athlete's personal thread (carnet#840), found the way the server
  // finds it: the thread flagged personal, else the newest one of their own the
  // list holds, else none — a look forges nothing, so Home opens on its empty
  // pane. The list is read through the page's own fetch, so a spec's list mock
  // — registered after this one — is the list it adopts from. "+" opens a
  // topic in that same thread, forging the one the server would when none.
  await page.route('**/api/chat/personal', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ conversation: await mockPersonalThread(page) }),
    });
  });
  await page.route('**/api/chat/personal/topics', async (route) => {
    const conversation = (await mockPersonalThread(page)) ?? mockForgedPersonalThread();
    await route.fulfill({
      status: 201,
      contentType: 'application/json',
      body: JSON.stringify({
        conversation,
        divider: {
          id: `divider-${Date.now()}`,
          role: 'system',
          content: '',
          finish_reason: 'divider',
          created_at: new Date().toISOString(),
        },
      }),
    });
  });

  // The use-case starters the empty thread offers (carnet#829), as the server
  // ranks them for an athlete with nothing connected yet, in their language.
  const starters = mockStarters(language);
  await page.route('**/api/chat/starters', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ starters }),
    });
  });

  // The read marker the open thread advances, and the mark-unread that clears
  // it. Registered after the list route so it wins the more specific path.
  await page.route('**/api/chat/conversations/*/read', async (route) => {
    await route.fulfill({ status: 204, body: '' });
  });

  // Mock user OAuth apps endpoint
  await page.route('**/api/users/oauth-apps', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ apps: [] }),
    });
  });

  // Mock A2A clients endpoint (used by A2AClientList in MCPTokensTab)
  await page.route('**/a2a/clients', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify([]),
    });
  });

  // Mock A2A client individual endpoints
  await page.route('**/a2a/clients/*', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({}),
    });
  });

  // Mock admin store stats endpoint (used by Coach Store tab badge)
  await page.route('**/api/admin/store/stats', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        pending_count: 0,
        published_count: 0,
        rejected_count: 0,
        total_installs: 0,
        rejection_rate: 0,
      }),
    });
  });

  // Mock the fuel-product review (super-admin tab): one pending proposal with
  // a stored label image, and a live catalogue of one product. The image is a
  // 1x1 PNG so the drawer's served-image path renders.
  await page.route('**/api/admin/fuel-products**', async (route) => {
    const url = new URL(route.request().url());
    if (url.pathname.endsWith('/source-image')) {
      await route.fulfill({
        status: 200,
        contentType: 'image/png',
        body: Buffer.from(
          'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=',
          'base64',
        ),
      });
      return;
    }
    const proposal = {
      id: '0b9f7c1e-5a3d-4e2f-9c8b-1a2b3c4d5e6f',
      product_id: null,
      brand: 'Maurten',
      name: 'Gel 100',
      variant: '',
      form: 'gel',
      gtin: '07350053630050',
      source_kind: 'shopify_ocr',
      source_url: 'https://shop.example/products/gel-100',
      has_source_image: true,
      source_image_mime: 'image/png',
      source_hash: 'sweep',
      facts: { serving_label: '1 gel (40 g)', serving_g: 40, energy_kcal: 100, carbs_g: 25, sugars_g: 12, glucose_fructose_ratio: 0.8 },
      provenance: { carbs_g: { source: 'label_ocr', confidence: 0.93 } },
      gate_flags: [],
      status: 'pending',
      proposed_by_user_id: null,
      reviewed_by_user_id: null,
      reviewed_at: null,
      reject_reason: null,
      reject_note: null,
      approved_version_id: null,
      created_at: '2026-10-29T12:00:00Z',
    };
    let body: unknown;
    if (url.pathname.endsWith('/api/admin/fuel-products')) {
      body = {
        limit: 100,
        offset: 0,
        products: [
          {
            product: { id: 'p-sweep', gtin: null, brand: 'SiS', name: 'Beta Fuel', variant: 'Orange', form: 'drink_mix', created_at: '2026-10-01T00:00:00Z' },
            current: {
              id: 'v-sweep',
              product_id: 'p-sweep',
              valid_from: '2026-10-20T00:00:00Z',
              superseded_by: null,
              facts: { carbs_g: 80, glucose_fructose_ratio: 0.8 },
              provenance: {},
              approved_by_user_id: 'user-123',
              approved_at: '2026-10-20T00:00:00Z',
              created_at: '2026-10-20T00:00:00Z',
            },
            version_count: 2,
          },
        ],
      };
    } else if (url.pathname.endsWith('/proposals')) {
      const status = url.searchParams.get('status') ?? 'pending';
      body = { proposals: status === 'pending' ? [proposal] : [], status, limit: 100, offset: 0 };
    } else {
      body = { proposal, source_image_bytes: 68 };
    }
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });
  });

  // Mock usage status endpoint (used by UsageWarningBanner in chat)
  await page.route('**/api/usage/status', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        daily: {
          messages: { allowed: true, current: 0, limit: 50, warning: false, burst_zone: false, resets_at: '2026-02-19T00:00:00Z' },
          tool_calls: { allowed: true, current: 0, limit: 100, warning: false, burst_zone: false, resets_at: '2026-02-19T00:00:00Z' },
          tokens: { allowed: true, current: 0, limit: 500000, warning: false, burst_zone: false, resets_at: '2026-02-19T00:00:00Z' },
        },
        weekly: {
          messages: { allowed: true, current: 0, limit: 250, warning: false, burst_zone: false, resets_at: '2026-02-23T00:00:00Z' },
          tool_calls: { allowed: true, current: 0, limit: 500, warning: false, burst_zone: false, resets_at: '2026-02-23T00:00:00Z' },
          tokens: { allowed: true, current: 0, limit: 2000000, warning: false, burst_zone: false, resets_at: '2026-02-23T00:00:00Z' },
        },
        resources: { agents: 0, max_agents: 3, conversations: 0, max_conversations: 20 },
      }),
    });
  });

  // Mock admin LLM consumption endpoint (used by LlmConsumptionPanel)
  await page.route('**/admin/usage/llm-consumption**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        summary: { total_tokens: 0, total_calls: 0, estimated_cost_usd: 0 },
        breakdown: [],
        daily_series: [],
      }),
    });
  });

  // Mock admin tool-usage endpoint (Tool Usage panel in the Analytics tab)
  await page.route('**/admin/tool-usage**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        summary: { total_invocations: 0, unique_tools: 0, turns_with_tools: 0 },
        breakdown: [],
        days: 30,
      }),
    });
  });

  // Mock providers status (needed by ChatTab and ProviderConnectionCards)
  // Default: no connected providers. A spec that needs to control this —
  // including controlling *when* the answer arrives — passes
  // `skipProvidersRoute` and registers its own.
  if (!skipProvidersRoute) {
    await page.route('**/api/providers', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ providers: [] }),
      });
    });
  }

  // Default onboarding status: a fully onboarded user. Specs exercising the
  // forced-onboarding flow override this with `needs_provider_connection: true`
  // via their own `page.route('**/api/me/onboarding-status', …)` call placed
  // BEFORE `loginToDashboard` so the override wins.
  await page.route('**/api/me/onboarding-status', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(ONBOARDED_STATUS),
    });
  });

  // Mock coaches (the chat header and the @handle palette read this list)
  await page.route('**/api/agents**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ agents: [], total: 0, metadata: { timestamp: new Date().toISOString(), api_version: 'v1' } }),
    });
  });

  // Mock an agent's version history (the edit sheet lists it): a never-edited
  // agent has none. Registered after the list mock so it wins for this sub-path.
  await page.route('**/api/agents/*/versions', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ versions: [], current_version: 0, total: 0 }),
    });
  });

  // Mock notifications (needed by sidebar NotificationBell + feed dropdown).
  // Pattern uses `notifications**` (no slash before **) so it also matches the
  // bare `/api/notifications?limit=10` feed query — the slash variant excluded
  // it and a 401 from the dev server fired the auth-failure event, logging the
  // user back out mid-test.
  await page.route('**/api/notifications**', async (route) => {
    const url = route.request().url();
    if (url.includes('unread-count')) {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ count: 0 }),
      });
    } else {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ data: [], total: 0, unread_count: 0 }),
      });
    }
  });

  // Mock store endpoints (needed by Discover tab)
  await page.route('**/api/store/**', async (route) => {
    const url = route.request().url();
    if (url.includes('/installations')) {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ agents: [], metadata: { timestamp: new Date().toISOString(), api_version: 'v1' } }),
      });
    } else if (url.includes('/categories')) {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ categories: [], metadata: { timestamp: new Date().toISOString(), api_version: 'v1' } }),
      });
    } else {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ agents: [], next_cursor: null, has_more: false, metadata: { timestamp: new Date().toISOString(), api_version: 'v1' } }),
      });
    }
  });

}

/**
 * How long to wait for the app shell to appear on first navigation.
 *
 * This is a wait on Vite's on-demand transform of the app, not an assertion
 * about the page. Playwright starts a fresh dev server per run
 * (`reuseExistingServer: false`) and several workers hit it cold at once, so
 * the first render routinely takes longer than the 10s this used to allow —
 * producing failures that moved between specs from run to run and looked like
 * real a11y regressions. Nothing here is asserted on time; a genuinely missing
 * form still fails, just after a wait that a cold machine can actually meet.
 *
 * Kept well inside the 60s per-test timeout in `playwright.config.ts`. Setting
 * the two equal is its own bug: one slow login consumes the entire budget and
 * the failure resurfaces further down as a click timing out on an element the
 * test was never going to reach.
 */
export const APP_SHELL_TIMEOUT_MS = 20000;

/**
 * The login screen's sign-in button. By test id, not by name: the label is
 * translated, so matching "Sign in" tied every spec to the chrome being
 * English, and the French sweeps could not sign in at all.
 */
export const SIGN_IN_BUTTON = '[data-testid="sign-in-button"]';

/**
 * Stand in for the server's hosted sign-in (carnet#787).
 *
 * The app sends the athlete to `/oauth2/authorize`; the real server shows its
 * login page there and, once the password is accepted, redirects back to the
 * `redirect_uri` the app sent with a code and the app's own `state`. This
 * answers that navigation with the redirect directly — or with `error` when a
 * spec needs a refused sign-in (`access_denied` is a suspended account). The
 * password form itself is the server's, tested on the Rust side.
 */
export async function mockHostedSignIn(page: Page, outcome: { error?: string } = {}) {
  await page.route('**/oauth2/authorize**', async (route) => {
    const request = new URL(route.request().url());
    const redirectUri = request.searchParams.get('redirect_uri');
    if (!redirectUri) {
      await route.fulfill({ status: 400, contentType: 'text/plain', body: 'redirect_uri missing' });
      return;
    }
    const callback = new URL(redirectUri);
    if (outcome.error) {
      callback.searchParams.set('error', outcome.error);
    } else {
      callback.searchParams.set('code', 'e2e-code');
    }
    callback.searchParams.set('state', request.searchParams.get('state') ?? '');
    await route.fulfill({ status: 302, headers: { location: callback.toString() } });
  });
}

/** Wait for the login screen: its sign-in button is painted. */
export async function waitForLoginScreen(page: Page, timeout = APP_SHELL_TIMEOUT_MS) {
  await page.locator(SIGN_IN_BUTTON).waitFor({ state: 'visible', timeout });
}

/**
 * Sign in from the login screen the way an athlete does: press the button,
 * pass through the (mocked) hosted sign-in, and return to the app once the
 * callback has redeemed its code and left `/auth/callback`.
 *
 * Installs the hosted-sign-in mock unless the spec already installed its own
 * (pass `mockHostedPage: false`), and expects `**\/oauth/token` to be mocked
 * for the code exchange — `setupDashboardMocks` does that.
 */
export async function signInThroughHostedPage(page: Page, options: { mockHostedPage?: boolean } = {}) {
  if (options.mockHostedPage !== false) {
    await mockHostedSignIn(page);
  }
  await waitForLoginScreen(page);
  await Promise.all([
    page.waitForURL((url) => url.pathname === '/auth/callback', { timeout: APP_SHELL_TIMEOUT_MS }),
    page.locator(SIGN_IN_BUTTON).click(),
  ]);
  await page.waitForURL((url) => url.pathname !== '/auth/callback', { timeout: APP_SHELL_TIMEOUT_MS });
}

/**
 * Signs in through the login screen and the mocked hosted sign-in.
 * Requires setupDashboardMocks() to be called first; the user is the one
 * setupDashboardMocks was given.
 */
export async function loginToDashboard(page: Page) {
  await page.goto('/');
  await signInThroughHostedPage(page);

  // Wait for dashboard to load - wait for main content area which only exists after successful login
  // Note: 'text=Dravr' would match login page's "Dravr" title, so use 'main' instead
  await page.waitForSelector('main', { timeout: APP_SHELL_TIMEOUT_MS });
  await page.waitForTimeout(300);
}

/**
 * Opens the athlete's own conversation the way a signed-in athlete does: it
 * is Home, where sign-in lands, so this waits for Home's personal surface —
 * pressing Home in the primary navigation first when another tab is open.
 * Only the visible one of the rail and the bottom bar is clicked.
 */
export async function openHome(page: Page) {
  if (!/#home(\/|$)/.test(new URL(page.url()).hash)) {
    await page
      .getByRole('navigation', { name: 'Primary navigation' })
      .getByRole('button', { name: 'Home', exact: true })
      .filter({ visible: true })
      .first()
      .click();
  }
  await page.waitForURL(/#home(\/|$)/, { timeout: APP_SHELL_TIMEOUT_MS });
  await page.getByTestId('home-page').waitFor({ timeout: APP_SHELL_TIMEOUT_MS });
}

/**
 * Opens one of the athlete's own conversations by title, the way an athlete
 * does on Home: the History sheet lists them, and picking a row opens it and
 * closes the sheet. Works whichever thread Home opened on.
 */
export async function openPersonalConversation(page: Page, title: string) {
  await openHome(page);
  await page.getByTestId('home-history-button').click();
  const history = page.getByRole('dialog', { name: 'History' });
  await history.getByTestId('conversation-row').filter({ hasText: title }).first().click();
  await history.waitFor({ state: 'hidden', timeout: APP_SHELL_TIMEOUT_MS });
}

/**
 * Opens the Groups tab — the rooms the athlete shares with other people — from
 * the primary navigation. The tab keeps the `chat` route.
 */
export async function openGroups(page: Page) {
  await page
    .getByRole('navigation', { name: 'Primary navigation' })
    .getByRole('button', { name: 'Groups', exact: true })
    .filter({ visible: true })
    .first()
    .click();
  await page.waitForURL(/#chat(\/|$)/, { timeout: APP_SHELL_TIMEOUT_MS });
}

/**
 * Navigates to a specific dashboard tab by clicking the sidebar button.
 */
export async function navigateToTab(page: Page, tabName: string) {
  // Try multiple selectors in order of preference:
  // 1. Button with span containing tab name (some UI versions)
  // 2. Button with generic/div containing tab name (current UI)
  // 3. Button containing the text anywhere (handles badges like "2 Users")
  // 4. Button with title attribute (collapsed sidebar)

  const selectors = [
    page.locator('button').filter({ has: page.locator(`span:has-text("${tabName}")`) }),
    page.locator('button').filter({ has: page.locator(`div:has-text("${tabName}")`) }),
    page.locator(`button:has-text("${tabName}")`),
    page.locator(`button[title="${tabName}"]`),
  ];

  for (const selector of selectors) {
    const isVisible = await selector.first().isVisible().catch(() => false);
    if (isVisible) {
      await selector.first().click();
      await page.waitForTimeout(300);
      return;
    }
  }

  // If none of the selectors worked, try clicking by accessible name (handles "2 Users" case)
  const buttonByName = page.getByRole('button', { name: new RegExp(`.*${tabName}.*`, 'i') });
  await buttonByName.click();
  await page.waitForTimeout(300);
}

/**
 * Shorthand for setting up mocks and logging in as an admin.
 */
export async function setupAndLoginAsAdmin(page: Page) {
  await setupDashboardMocks(page, { role: 'admin' });
  await loginToDashboard(page);
}

/**
 * Shorthand for setting up mocks and logging in as a super admin.
 */
export async function setupAndLoginAsSuperAdmin(page: Page) {
  await setupDashboardMocks(page, { role: 'super_admin' });
  await loginToDashboard(page);
}

/**
 * Shorthand for setting up mocks and logging in as a regular user.
 */
export async function setupAndLoginAsUser(page: Page) {
  await setupDashboardMocks(page, { role: 'user' });
  await loginToDashboard(page);
}

/** Getting started for an athlete whose first steps were all done weeks ago: the section is gone. */
const GETTING_STARTED_DONE = {
  rows: ['connect', 'meet_team', 'last_workout', 'about_me_more', 'check_in'].map((id, index) => ({
    id,
    order: index + 1,
    eligible: true,
    done: true,
    current: false,
    postback: index === 0 ? null : `gs:${index + 1}:${id}`,
  })),
  done_count: 5,
  total: 5,
  folded_at: '2026-01-05T08:00:00Z',
  hidden: false,
};

/** Getting started on a first arrival: data connected, meeting the team next, no check-in data yet. */
const GETTING_STARTED_FIRST_ARRIVAL = {
  rows: [
    { id: 'connect', order: 1, eligible: true, done: true, current: false, postback: null },
    { id: 'meet_team', order: 2, eligible: true, done: false, current: true, postback: 'gs:2:meet_team' },
    { id: 'last_workout', order: 3, eligible: true, done: false, current: false, postback: 'gs:3:last_workout' },
    { id: 'about_me_more', order: 4, eligible: true, done: false, current: false, postback: 'gs:4:about_me_more' },
    { id: 'check_in', order: 5, eligible: true, done: false, current: false, postback: 'gs:5:check_in' },
  ],
  done_count: 1,
  total: 5,
  folded_at: null,
  hidden: false,
};

/** The first-arrival team: the lead the thread is bound to and the two members the rules ticked. */
const FIRST_ARRIVAL_TEAM = [
  { id: 'agent-endurance', title: 'Endurance', handle: 'endurance', description: 'Plans and reads every session', category: 'training' },
  { id: 'agent-fuelling', title: 'Fuelling', handle: 'fuelling', description: 'long days, race food', category: 'nutrition' },
  { id: 'agent-recovery', title: 'Recovery', handle: 'recovery', description: 'sleep, HRV, load', category: 'recovery' },
].map((agent) => ({
  ...agent,
  system_prompt: '',
  tags: [],
  token_count: 0,
  is_favorite: false,
  use_count: 0,
  last_used_at: null,
  created_at: '2026-10-01T00:00:00Z',
  updated_at: '2026-10-01T00:00:00Z',
  is_system: true,
  is_assigned: true,
}));

/**
 * A first arrival on Home (carnet#1098): Getting started open on Meet your
 * team, a team of three led by Endurance, a daily check-in. Call after
 * `setupDashboardMocks` so these routes win.
 */
export async function mockHomeFirstArrival(page: Page): Promise<void> {
  await page.route('**/api/me/getting-started', async (route) => {
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(GETTING_STARTED_FIRST_ARRIVAL) });
  });
  await page.route('**/api/me/check-in', async (route) => {
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ cadence: 'daily' }) });
  });
  await page.route('**/api/me/onboarding-status', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ ...ONBOARDED_STATUS, pillars_covered: 2, pillars_total: 7 }),
    });
  });
  await page.route('**/api/agents**', async (route) => {
    const url = new URL(route.request().url());
    if (url.pathname !== '/api/agents') {
      await route.fallback();
      return;
    }
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        agents: FIRST_ARRIVAL_TEAM,
        total: FIRST_ARRIVAL_TEAM.length,
        metadata: { timestamp: new Date().toISOString(), api_version: 'v1' },
      }),
    });
  });
  await page.route('**/api/chat/personal', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        conversation: {
          id: 'conv-home',
          title: 'Endurance',
          agent_id: 'agent-endurance',
          personal: true,
          channel_type: 'web',
          unread_count: 0,
          created_at: '2026-10-21T07:40:00Z',
          updated_at: '2026-10-21T07:42:00Z',
        },
      }),
    });
  });
}
