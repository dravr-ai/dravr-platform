// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Playwright E2E tests for the user-facing BillingPage (Phase 5E).
// ABOUTME: Covers Starter fallback rendering, quota display, checkout POST + CSRF, portal mutation.

import { test, expect, type Page, type Route } from '@playwright/test';

import { setupDashboardMocks, loginToDashboard } from './test-helpers';

interface MockOptions {
  /** When true, GET /api/billing/subscription returns 404 (Starter fallback). */
  noSubscription?: boolean;
  /** Override the quota counters returned by /api/users/me/quota. */
  quota?: {
    counters: Array<{
      counter_type: string;
      current: number;
      limit: number;
      warning: boolean;
      burst_zone: boolean;
      resets_at: string;
    }>;
  };
}

async function setupBillingMocks(page: Page, opts: MockOptions = {}) {
  const noSubscription = opts.noSubscription ?? true;

  await page.route('**/api/billing/subscription', async (route: Route) => {
    if (noSubscription) {
      await route.fulfill({
        status: 404,
        contentType: 'application/json',
        body: JSON.stringify({ error: 'no subscription record yet' }),
      });
    } else {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          id: 'sub-row-1',
          tenant_id: 'tenant-1',
          user_id: 'user-123',
          provider: 'dummy',
          provider_customer_id: 'cus_test_abc123',
          provider_subscription_id: 'sub_test_abc123',
          status: 'active',
          plan_tier: 'professional',
          current_period_start: '2026-04-01T00:00:00Z',
          current_period_end: '2026-05-01T00:00:00Z',
          cancel_at_period_end: false,
        }),
      });
    }
  });

  await page.route('**/api/users/me/quota', async (route: Route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(
        opts.quota ?? {
          counters: [
            {
              counter_type: 'daily_messages',
              current: 0,
              limit: 50,
              warning: false,
              burst_zone: false,
              resets_at: '2026-04-29T00:00:00Z',
            },
            {
              counter_type: 'daily_tokens',
              current: 0,
              limit: 500_000,
              warning: false,
              burst_zone: false,
              resets_at: '2026-04-29T00:00:00Z',
            },
            {
              counter_type: 'weekly_tokens',
              current: 0,
              limit: 2_500_000,
              warning: false,
              burst_zone: false,
              resets_at: '2026-05-04T00:00:00Z',
            },
            {
              counter_type: 'daily_tool_calls',
              current: 0,
              limit: 200,
              warning: false,
              burst_zone: false,
              resets_at: '2026-04-29T00:00:00Z',
            },
            {
              counter_type: 'daily_conversations',
              current: 0,
              limit: 10,
              warning: false,
              burst_zone: false,
              resets_at: '2026-04-29T00:00:00Z',
            },
            {
              counter_type: 'active_coaches',
              current: 0,
              limit: 3,
              warning: false,
              burst_zone: false,
              resets_at: '2026-04-29T00:00:00Z',
            },
          ],
        },
      ),
    });
  });

  await page.route('**/api/billing/invoices', async (route: Route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ invoices: [] }),
    });
  });
}

test.describe('Billing - Starter fallback (no subscription on file)', () => {
  test.beforeEach(async ({ page }) => {
    await setupDashboardMocks(page, { role: 'user', email: 'webtest@pierre.dev' });
    // Override the helper's default tier:'professional' — Starter fallback requires
    // a starter user so BillingPage's `sub?.plan_tier ?? user?.tier ?? 'starter'`
    // resolves to "Starter" when /api/billing/subscription returns 404.
    await page.route('**/oauth/token', async (route: Route) => {
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
            email: 'webtest@pierre.dev',
            display_name: 'Test User',
            role: 'user',
            is_admin: false,
            user_status: 'active',
            tier: 'starter',
            tenant_id: 'user-123',
          },
        }),
      });
    });
    await setupBillingMocks(page, { noSubscription: true });
    await loginToDashboard(page);
    // <main> and <aside> exist on Login.tsx too — only <nav> is dashboard-exclusive.
    await page.waitForSelector('nav', { timeout: 10000 });
  });

  test('renders Current Plan card with Starter badge and upgrade CTAs', async ({ page }) => {
    await page.locator('button', { hasText: 'Billing' }).first().click();
    await expect(page.getByRole('heading', { name: 'Current Plan' })).toBeVisible({ timeout: 5000 });
    await expect(page.getByText('Starter', { exact: true }).first()).toBeVisible();
    await expect(page.getByText("No subscription on file", { exact: false })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Upgrade to Professional' })).toBeVisible();
    await expect(page.getByRole('button', { name: /Talk to Sales \(Enterprise\)/i })).toBeVisible();
  });

  test('renders all six quota counters with limits', async ({ page }) => {
    await page.locator('button', { hasText: 'Billing' }).first().click();
    await expect(page.getByRole('heading', { name: 'Usage Quota' })).toBeVisible({ timeout: 5000 });

    for (const label of [
      'Daily Messages',
      'Daily Tokens',
      'Weekly Tokens',
      'Daily Tool Calls',
      'Daily Conversations',
      'Active Coaches',
    ]) {
      await expect(page.getByText(label, { exact: false })).toBeVisible();
    }
  });

  test('upgrade button POSTs /api/billing/checkout with body + redirects on success', async ({ page }) => {
    let capturedBody: unknown = null;

    await page.route('**/api/billing/checkout', async (route: Route) => {
      capturedBody = JSON.parse(route.request().postData() ?? '{}');
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ checkout_url: 'https://example.test/checkout/test_session_xyz' }),
      });
    });

    await page.locator('button', { hasText: 'Billing' }).first().click();
    await expect(page.getByRole('button', { name: 'Upgrade to Professional' })).toBeVisible({ timeout: 5000 });

    // Capture the redirect that BillingPage triggers via `window.location.href = …`.
    // Modern Chromium makes `window.location.href` non-configurable, so we intercept
    // the actual navigation request instead by mocking the destination domain.
    let checkoutNavigation: string | null = null;
    await page.route('https://example.test/checkout/**', async (route: Route) => {
      checkoutNavigation = route.request().url();
      await route.fulfill({
        status: 200,
        contentType: 'text/html',
        body: '<html><body>checkout stub</body></html>',
      });
    });

    await page.getByRole('button', { name: 'Upgrade to Professional' }).click();

    await expect.poll(() => checkoutNavigation, { timeout: 5000 }).toBe(
      'https://example.test/checkout/test_session_xyz',
    );

    // The request body must carry the tier + user_id the server-side handler expects.
    const body = capturedBody as Record<string, unknown> | null;
    expect(body).not.toBeNull();
    expect(body!.tier).toBe('professional');
    expect(body!.user_id).toBe('user-123');
    expect(body!.success_url).toContain('/billing?upgrade=success');
    expect(body!.cancel_url).toContain('/billing?upgrade=cancel');
  });

  test('checkout POST carries X-CSRF-Token header (regression: CSRF middleware would reject without it)', async ({ page }) => {
    let capturedHeaders: Record<string, string> = {};

    await page.route('**/api/billing/checkout', async (route: Route) => {
      capturedHeaders = route.request().headers();
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ checkout_url: 'https://example.test/checkout/test_csrf' }),
      });
    });

    await page.locator('button', { hasText: 'Billing' }).first().click();
    await expect(page.getByRole('button', { name: 'Upgrade to Professional' })).toBeVisible({ timeout: 5000 });
    await page.getByRole('button', { name: 'Upgrade to Professional' }).click();

    await expect.poll(() => capturedHeaders['x-csrf-token']).toBeTruthy();
  });
});

test.describe('Billing - Existing Professional subscription', () => {
  test.beforeEach(async ({ page }) => {
    await setupDashboardMocks(page, { role: 'user', email: 'pro@pierre.dev' });
    await setupBillingMocks(page, { noSubscription: false });
    await loginToDashboard(page);
    await page.waitForSelector('nav', { timeout: 10000 });
  });

  test('shows subscription details + Manage Subscription button', async ({ page }) => {
    await page.locator('button', { hasText: 'Billing' }).first().click();
    await expect(page.getByRole('heading', { name: 'Current Plan' })).toBeVisible({ timeout: 5000 });
    // Provider customer id should be rendered in the details grid.
    await expect(page.getByText('cus_test_abc123', { exact: false })).toBeVisible();
    await expect(page.getByRole('button', { name: /Manage Subscription/i })).toBeVisible();
  });

  test('Manage Subscription POSTs /api/billing/portal and redirects to the hosted portal', async ({ page }) => {
    let portalBody: Record<string, unknown> | null = null;

    await page.route('**/api/billing/portal', async (route: Route) => {
      portalBody = JSON.parse(route.request().postData() ?? '{}');
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ portal_url: 'https://example.test/portal/test_portal_xyz' }),
      });
    });

    let portalNavigation: string | null = null;
    await page.route('https://example.test/portal/**', async (route: Route) => {
      portalNavigation = route.request().url();
      await route.fulfill({
        status: 200,
        contentType: 'text/html',
        body: '<html><body>portal stub</body></html>',
      });
    });

    await page.locator('button', { hasText: 'Billing' }).first().click();
    await expect(page.getByRole('button', { name: /Manage Subscription/i })).toBeVisible({ timeout: 5000 });
    await page.getByRole('button', { name: /Manage Subscription/i }).click();

    await expect.poll(() => portalNavigation, { timeout: 5000 }).toBe(
      'https://example.test/portal/test_portal_xyz',
    );
    expect(portalBody!.provider_customer_id).toBe('cus_test_abc123');
    expect(portalBody!.return_url).toContain('/billing');
  });
});
