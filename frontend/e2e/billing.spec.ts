// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Playwright E2E tests for the user-facing BillingPage (Phase 5E).
// ABOUTME: Covers Starter fallback rendering, quota display, checkout POST + CSRF, portal mutation.

import { test, expect, type Route } from '@playwright/test';

import { setupBillingMocks } from './billing-mocks';
import { setupDashboardMocks, loginToDashboard } from './test-helpers';

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
    await page.locator('button', { hasText: 'Usage' }).first().click();
    await expect(page.getByRole('heading', { name: 'Current Plan' })).toBeVisible({ timeout: 5000 });
    await expect(page.getByText('Starter', { exact: true }).first()).toBeVisible();
    await expect(page.getByText("No subscription on file", { exact: false })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Upgrade to Professional' })).toBeVisible();
    await expect(page.getByRole('button', { name: /Talk to Sales \(Enterprise\)/i })).toBeVisible();
  });

  test('renders all six quota counters with limits', async ({ page }) => {
    await page.locator('button', { hasText: 'Usage' }).first().click();
    await expect(page.getByRole('heading', { name: 'Usage Quota' })).toBeVisible({ timeout: 5000 });

    for (const label of [
      'Daily Messages',
      'Daily Tokens',
      'Weekly Tokens',
      'Daily Tool Calls',
      'Daily Conversations',
      'Active agents',
    ]) {
      await expect(page.getByText(label, { exact: false })).toBeVisible();
    }
  });

  test('renders the plan comparison with all three tiers and a Current badge', async ({ page }) => {
    await page.locator('button', { hasText: 'Usage' }).first().click();
    await expect(page.getByRole('heading', { name: 'Plans' })).toBeVisible({ timeout: 5000 });
    // The three tier headings inside the Plans grid.
    await expect(page.getByRole('heading', { name: 'Starter', exact: true })).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Professional', exact: true })).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Enterprise', exact: true })).toBeVisible();
    // Feature rows sourced from the plans endpoint.
    await expect(page.getByText('Messages / day', { exact: false }).first()).toBeVisible();
    await expect(page.getByText('Included usage', { exact: false }).first()).toBeVisible();
    // Starter is the current plan → its card shows a Current-plan button.
    await expect(page.getByRole('button', { name: 'Current plan' })).toBeVisible();
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

    await page.locator('button', { hasText: 'Usage' }).first().click();
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

    // The request body carries the tier + redirect targets; the user and
    // tenant come from the bearer token, so the body must not name them.
    const body = capturedBody as Record<string, unknown> | null;
    expect(body).not.toBeNull();
    expect(body!.tier).toBe('professional');
    expect(body).not.toHaveProperty('user_id');
    expect(body).not.toHaveProperty('tenant_id');
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

    await page.locator('button', { hasText: 'Usage' }).first().click();
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
    await page.locator('button', { hasText: 'Usage' }).first().click();
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

    await page.locator('button', { hasText: 'Usage' }).first().click();
    await expect(page.getByRole('button', { name: /Manage Subscription/i })).toBeVisible({ timeout: 5000 });
    await page.getByRole('button', { name: /Manage Subscription/i }).click();

    await expect.poll(() => portalNavigation, { timeout: 5000 }).toBe(
      'https://example.test/portal/test_portal_xyz',
    );
    // The customer is the caller's own subscription row, resolved server-side.
    expect(portalBody).not.toHaveProperty('provider_customer_id');
    expect(portalBody!.return_url).toContain('/billing');
  });
});

test.describe('Billing - Payment problem (dunning)', () => {
  test.beforeEach(async ({ page }) => {
    await setupDashboardMocks(page, { role: 'user', email: 'pro@pierre.dev' });
    await setupBillingMocks(page, { noSubscription: false, subscriptionStatus: 'past_due' });
    await loginToDashboard(page);
    await page.waitForSelector('nav', { timeout: 10000 });
  });

  test('shows the payment-problem banner with an Update payment CTA', async ({ page }) => {
    await page.locator('button', { hasText: 'Usage' }).first().click();
    await expect(
      page.getByRole('heading', { name: /Payment problem/i })
    ).toBeVisible({ timeout: 5000 });
    await expect(page.getByRole('button', { name: 'Update payment' })).toBeVisible();
  });

  test('Update payment opens the billing portal', async ({ page }) => {
    await page.route('**/api/billing/portal', async (route: Route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ portal_url: 'https://example.test/portal/dunning_fix' }),
      });
    });
    let portalNavigation: string | null = null;
    await page.route('https://example.test/portal/**', async (route: Route) => {
      portalNavigation = route.request().url();
      await route.fulfill({ status: 200, contentType: 'text/html', body: '<html></html>' });
    });

    await page.locator('button', { hasText: 'Usage' }).first().click();
    await expect(page.getByRole('button', { name: 'Update payment' })).toBeVisible({ timeout: 5000 });
    await page.getByRole('button', { name: 'Update payment' }).click();

    await expect.poll(() => portalNavigation, { timeout: 5000 }).toBe(
      'https://example.test/portal/dunning_fix',
    );
  });
});

test.describe('Billing - degraded payloads keep the app mounted', () => {
  test.beforeEach(async ({ page }) => {
    await setupDashboardMocks(page, { role: 'user', email: 'webtest@pierre.dev' });
    // Every billing endpoint answers 200 with an envelope and no array — the
    // shape the design sweep hit when it walked Usage. Each block guarded
    // `query.data` and then read `.plans` / `.counters` / `.invoices` off it,
    // so this payload took the "data present" branch and threw, unmounting the
    // whole Dashboard into the ErrorBoundary. The fallback copy asserted below
    // was already written; nothing could reach it.
    for (const path of [
      '**/api/billing/subscription',
      '**/api/users/me/quota',
      '**/api/billing/invoices',
      '**/api/billing/plans',
    ]) {
      await page.route(path, async (route: Route) => {
        await route.fulfill({ status: 200, contentType: 'application/json', body: '{}' });
      });
    }
    await loginToDashboard(page);
    await page.waitForSelector('nav', { timeout: 10000 });
  });

  test('shows the written fallbacks instead of crashing into the ErrorBoundary', async ({ page }) => {
    await page.locator('button', { hasText: 'Usage' }).first().click();

    await expect(page.getByRole('heading', { name: 'Plans' })).toBeVisible({ timeout: 5000 });
    await expect(page.getByText('Plan information unavailable.')).toBeVisible();
    await expect(page.getByText('Quota information unavailable.')).toBeVisible();
    await expect(page.getByText('No invoices yet.')).toBeVisible();

    // The dashboard shell is still mounted — the ErrorBoundary replaces it
    // outright, so its absence is the regression this pins.
    await expect(page.locator('[data-page-shell]')).toBeVisible();
    await expect(page.getByText('Something went wrong')).toHaveCount(0);
  });
});
