// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Shared test helper functions for Playwright E2E tests.
// ABOUTME: Provides reusable authentication mocks and login helpers.

import type { Page } from '@playwright/test';

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
}

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
  await page.route('**/api/me/activities/recent**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ activities: [], as_of: null, stale: false }),
    });
  });
  await page.route('**/api/me/activities/*/*/route', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ route: null, reason: 'no_gps' }),
    });
  });

  // Default onboarding status for spec-local login helpers that skip
  // `setupDashboardMocks`. Specs exercising the forced-onboarding flow
  // override with `needs_provider_connection: true` before calling login.
  await page.route('**/api/me/onboarding-status', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ needs_provider_connection: false }),
    });
  });
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
      body: JSON.stringify({ needs_provider_connection: false }),
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

  // Mock user LLM settings (read by the About pane for the model line)
  await page.route('**/api/user/llm-settings**', async (route) => {
    if (route.request().method() === 'GET') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          current_provider: null,
          providers: [],
          user_credentials: [],
          tenant_credentials: [],
          system_provider: {
            name: 'copilot_headless',
            display_name: 'Copilot Headless',
            model: 'claude-sonnet-5',
          },
        }),
      });
    } else {
      await route.fallback();
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
