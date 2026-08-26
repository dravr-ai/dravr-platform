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

  // Mock OAuth2 ROPC login endpoint
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

  // Mock dashboard overview
  await page.route('**/api/dashboard/overview', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        total_api_keys: 10,
        active_api_keys: 8,
        total_requests_today: 450,
        total_requests_this_month: 12500,
      }),
    });
  });

  // Mock rate limits
  await page.route('**/api/dashboard/rate-limits', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify([]),
    });
  });

  // Mock A2A dashboard
  await page.route('**/a2a/dashboard/overview', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        total_clients: 5,
        active_clients: 3,
        requests_today: 100,
        requests_this_month: 3000,
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
      body: JSON.stringify({ count: 0, users: [] }),
    });
  });

  // Mock admin users list
  await page.route('**/api/admin/users**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ users: [], total_count: 0 }),
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

  // Mock OAuth status endpoint (used by Connections tab)
  // Note: Backend returns array directly, getStatus() wraps it in { providers: ... }
  await page.route('**/api/oauth/status', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify([]),
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

  // Mock chat conversations endpoint
  await page.route('**/api/chat/conversations**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ conversations: [], total: 0, limit: 50, offset: 0 }),
    });
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
        resources: { coaches: 0, max_coaches: 3, conversations: 0, max_conversations: 20 },
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

  // Mock user LLM consumption endpoint
  await page.route('**/api/usage/llm-consumption**', async (route) => {
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

  // Mock coaches (needed by PromptSuggestions in welcome view)
  await page.route('**/api/coaches**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ coaches: [], total: 0, metadata: { timestamp: new Date().toISOString(), api_version: 'v1' } }),
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
        body: JSON.stringify({ coaches: [], metadata: { timestamp: new Date().toISOString(), api_version: 'v1' } }),
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
        body: JSON.stringify({ coaches: [], next_cursor: null, has_more: false, metadata: { timestamp: new Date().toISOString(), api_version: 'v1' } }),
      });
    }
  });

  // Mock user LLM settings (needed by AI Settings tab)
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
 * Performs login through the login form.
 * Requires setupDashboardMocks() to be called first.
 */
export async function loginToDashboard(page: Page, credentials?: { email?: string; password?: string }) {
  const { email = 'admin@test.com', password = 'password123' } = credentials || {};

  await page.goto('/');
  await page.waitForSelector('form', { timeout: APP_SHELL_TIMEOUT_MS });
  await page.locator('input[name="email"]').fill(email);
  await page.locator('input[name="password"]').fill(password);
  await page.getByRole('button', { name: 'Sign in' }).click();

  // Wait for dashboard to load - wait for main content area which only exists after successful login
  // Note: 'text=Dravr' would match login page's "Dravr" title, so use 'main' instead
  await page.waitForSelector('main', { timeout: APP_SHELL_TIMEOUT_MS });
  await page.waitForTimeout(300);
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
