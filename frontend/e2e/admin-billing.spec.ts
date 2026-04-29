// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Playwright E2E for admin billing surface — UserDetailDrawer Usage Card + BillingTab CSV export.
// ABOUTME: Regression-pins: Usage Card must coexist with Impersonate button; admin /api/admin/users/{id}/usage path.

import { test, expect, type Page, type Route } from '@playwright/test';

async function loginAsSuperAdminWithUsers(page: Page) {
  await page.route('**/admin/setup/status', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ needs_setup: false, admin_user_exists: true }),
    });
  });

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
          user_id: 'super-admin-1',
          email: 'superadmin@test.com',
          display_name: 'Super Admin',
          role: 'super_admin',
          is_admin: true,
          user_status: 'active',
          tier: 'enterprise',
        },
      }),
    });
  });

  await page.route('**/api/dashboard/overview', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        total_api_keys: 5,
        active_api_keys: 3,
        total_requests_today: 150,
        total_requests_this_month: 2500,
      }),
    });
  });

  await page.route('**/api/dashboard/rate-limits', async (route) => {
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify([]) });
  });

  await page.route('**/a2a/dashboard/overview', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ total_clients: 0, active_sessions: 0, requests_today: 0, error_rate: 0 }),
    });
  });

  await page.route('**/api/dashboard/analytics**', async (route) => {
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ daily_usage: [] }) });
  });

  await page.route('**/api/admin/pending-users', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ count: 0, users: [] }),
    });
  });

  await page.route('**/api/admin/users**', async (route) => {
    // Only fulfill the listing endpoint; the per-user routes are matched separately below.
    const url = route.request().url();
    if (url.includes('/usage') || url.includes('/cost-timeseries')) {
      await route.fallback();
      return;
    }
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        users: [
          {
            id: 'user-active-1',
            email: 'testuser@example.com',
            display_name: 'Test User',
            role: 'user',
            user_status: 'active',
            tier: 'starter',
            created_at: '2024-01-15T10:00:00Z',
            last_active: '2024-01-20T15:30:00Z',
          },
        ],
        total_count: 1,
      }),
    });
  });

  await page.route('**/admin/users/*/rate-limit', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        user_id: 'user-active-1',
        tier: 'starter',
        rate_limits: {
          daily: { limit: 1000, used: 50, remaining: 950 },
          monthly: { limit: 10000, used: 500, remaining: 9500 },
        },
        reset_times: {
          daily_reset: '2024-01-21T00:00:00Z',
          monthly_reset: '2024-02-01T00:00:00Z',
        },
      }),
    });
  });

  await page.route('**/admin/users/*/activity**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ user_id: 'user-active-1', period_days: 30, total_requests: 0, top_tools: [] }),
    });
  });

  // NotificationBell fires `/api/notifications?limit=10` on every Dashboard render.
  // Without a 200 here the dev server returns 401 → AuthContext fires the auth-failure
  // event and logs the user back out before the test can interact with the UI.
  await page.route('**/api/notifications**', async (route) => {
    if (route.request().url().includes('unread-count')) {
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

  // Coaches list (PromptSuggestions on first dashboard render).
  await page.route('**/api/coaches**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ coaches: [], total: 0, metadata: { timestamp: new Date().toISOString(), api_version: 'v1' } }),
    });
  });

  // Admin store stats (Coach Store badge in admin sidebar).
  await page.route('**/api/admin/store/stats', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ pending_count: 0, published_count: 0, rejected_count: 0, total_installs: 0, rejection_rate: 0 }),
    });
  });

  await page.goto('/');
  await page.waitForSelector('form');
  await page.locator('input[name="email"]').fill('superadmin@test.com');
  await page.locator('input[name="password"]').fill('password123');
  await page.getByRole('button', { name: 'Sign in' }).click();
  // Login.tsx has its own "Dravr" splash text — gate on <nav> which is dashboard-exclusive.
  await page.waitForSelector('nav', { timeout: 10000 });
  await page.waitForTimeout(300);
}

async function openUserDetailDrawer(page: Page) {
  await page.locator('button', { hasText: 'Users' }).first().click();
  await page.waitForTimeout(300);
  await page.locator('button', { hasText: 'Active' }).first().click();
  await page.waitForTimeout(300);
  await page.getByText('testuser@example.com').first().click();
  await page.waitForTimeout(500);
}

test.describe('Admin Billing - UserDetailDrawer Usage Card', () => {
  test('renders Usage Card with by_model rows alongside Impersonate button', async ({ page }) => {
    // Well-formed usage payload — the canonical happy path.
    await page.route('**/api/admin/users/*/usage**', async (route: Route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          user_id: 'user-active-1',
          from: '2026-04-01T00:00:00Z',
          by_model: [
            {
              provider: 'google',
              model: 'gemini-2.5-flash',
              call_type: 'chat',
              total_tokens: 12_500,
              prompt_tokens: 9_500,
              completion_tokens: 3_000,
              calls: 42,
            },
          ],
          daily: [],
        }),
      });
    });

    await loginAsSuperAdminWithUsers(page);
    await openUserDetailDrawer(page);

    // Usage Card is the heading "Usage (Month-to-Date)".
    await expect(page.getByText(/Usage \(Month-to-Date\)/i)).toBeVisible();
    // by_model row data should render.
    await expect(page.getByText('google/gemini-2.5-flash')).toBeVisible();
    await expect(page.getByText('12,500 tok')).toBeVisible();

    // Regression for the original bug (Usage Card crash unmounted the drawer):
    // the Impersonate User button further down in the same drawer must still render.
    await expect(page.getByRole('button', { name: /Impersonate User/i })).toBeVisible();
  });

  test('renders empty state + Impersonate button when by_model is empty array', async ({ page }) => {
    await page.route('**/api/admin/users/*/usage**', async (route: Route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ user_id: 'user-active-1', from: '2026-04-01T00:00:00Z', by_model: [], daily: [] }),
      });
    });

    await loginAsSuperAdminWithUsers(page);
    await openUserDetailDrawer(page);

    await expect(page.getByText('No LLM activity this month')).toBeVisible();
    await expect(page.getByRole('button', { name: /Impersonate User/i })).toBeVisible();
  });

  test('regression: drawer survives malformed usage payload (no by_model field)', async ({ page }) => {
    // Simulates the pre-fix bug: a route mock returning the wrong shape (e.g. user-list payload)
    // would crash usage.by_model.reduce(...) and unmount the entire drawer.
    await page.route('**/api/admin/users/*/usage**', async (route: Route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ users: [], total_count: 0 }),
      });
    });

    await loginAsSuperAdminWithUsers(page);
    await openUserDetailDrawer(page);

    // Drawer must render the empty-state branch, NOT crash. The Impersonate button is the canary.
    await expect(page.getByText('No LLM activity this month')).toBeVisible();
    await expect(page.getByRole('button', { name: /Impersonate User/i })).toBeVisible();
  });
});

test.describe('Admin Billing - per-user usage route prefix', () => {
  test('drawer hits /api/admin/users/{id}/usage (not /admin/users/{id}/usage)', async ({ page }) => {
    let hitApiPrefixed = false;
    let hitNonPrefixed = false;

    await page.route('**/api/admin/users/*/usage**', async (route: Route) => {
      hitApiPrefixed = true;
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ user_id: 'user-active-1', from: '2026-04-01T00:00:00Z', by_model: [], daily: [] }),
      });
    });

    // Catch the broken pre-fix path explicitly.
    await page.route('**/admin/users/*/usage**', async (route: Route) => {
      // If any request lands on the un-prefixed path (the original bug), flag it.
      if (!route.request().url().includes('/api/admin/')) {
        hitNonPrefixed = true;
      }
      await route.fallback();
    });

    await loginAsSuperAdminWithUsers(page);
    await openUserDetailDrawer(page);

    // Wait for the drawer to settle so the useQuery has time to fire.
    await expect(page.getByText('No LLM activity this month')).toBeVisible();

    expect(hitApiPrefixed).toBe(true);
    expect(hitNonPrefixed).toBe(false);
  });
});
