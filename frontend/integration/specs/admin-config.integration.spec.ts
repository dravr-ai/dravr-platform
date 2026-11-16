// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Integration tests for Admin Configuration with real backend.
// ABOUTME: Verifies configuration API endpoints and data persistence.

import { test, expect, type Page } from '@playwright/test';
import {
  createAndLoginAsAdmin,
  createAndLoginTestUser,
  navigateToTab,
  waitForDashboardLoad,
  getBackendUrl,
} from '../helpers';
import { testUsers, timeouts } from '../fixtures';

/**
 * The header a signed-in browser sends on every write. The session is the
 * httpOnly cookie `page.request` already carries; the server refuses a
 * cookie-authenticated write without this token with 401, before the handler
 * reads the body. The SPA keeps the token where `@pierre/api-client` reads it.
 */
async function csrfHeader(page: Page): Promise<Record<string, string>> {
  const token = await page.evaluate(() => localStorage.getItem('pierre_csrf_token'));
  expect(token, 'the signed-in session holds a CSRF token').toBeTruthy();
  return { 'X-CSRF-Token': token ?? '' };
}

/**
 * End the signed-in session and return the write header it held.
 *
 * A write test that expects 401 for a missing session has to send this
 * header: without it the server answers a cookie-authenticated write 401 for
 * the missing CSRF token, and the test passes whether or not the session was
 * cleared. With it, a write whose session is still there reaches the handler
 * and is not a 401. The cookie jar is asserted empty so the 401 is the
 * missing session's.
 */
async function endSessionKeepingCsrfHeader(page: Page): Promise<Record<string, string>> {
  const headers = await csrfHeader(page);
  await page.context().clearCookies();
  await page.evaluate(() => localStorage.removeItem('pierre_auth_token'));
  expect(await page.context().cookies(), 'no session cookie is left').toHaveLength(0);
  return headers;
}

/** One toggle per row of the tool table: drawn only once the tool list has loaded. */
function toolToggles(page: Page) {
  return page.locator('tbody [role="switch"]');
}

test.describe('Admin Configuration Integration Tests', () => {
  test.beforeEach(async ({ page }) => {
    const loginResult = await createAndLoginAsAdmin(page);
    expect(loginResult.success).toBe(true);
    await waitForDashboardLoad(page);
  });

  test.describe('Configuration Catalog API', () => {
    test('backend returns valid configuration catalog', async ({ page }) => {
      const backendUrl = getBackendUrl();
      const response = await page.request.get(`${backendUrl}/api/admin/config/catalog`);

      // API should return success
      expect(response.ok()).toBe(true);

      const data = await response.json();
      expect(data.success).toBe(true);
      expect(data.data).toBeDefined();
      expect(data.data.categories).toBeDefined();
      expect(Array.isArray(data.data.categories)).toBe(true);
      // An admin sees every category, and the catalog always has some.
      expect(data.data.categories.length).toBeGreaterThan(0);
    });

    test('catalog contains expected parameter structure', async ({ page }) => {
      const backendUrl = getBackendUrl();
      const response = await page.request.get(`${backendUrl}/api/admin/config/catalog`);

      const data = await response.json();
      const categories: Array<{ parameters: unknown[] }> = data.data.categories;
      expect(categories.length).toBeGreaterThan(0);

      // Every category has the required fields
      for (const category of categories) {
        expect(category).toHaveProperty('name');
        expect(category).toHaveProperty('display_name');
        expect(category).toHaveProperty('parameters');
        expect(Array.isArray(category.parameters)).toBe(true);
      }

      // The catalog is never without parameters, and each has the required fields
      const parameters = categories.flatMap((category) => category.parameters);
      expect(parameters.length).toBeGreaterThan(0);
      for (const param of parameters) {
        expect(param).toHaveProperty('key');
        expect(param).toHaveProperty('display_name');
        expect(param).toHaveProperty('data_type');
        expect(param).toHaveProperty('current_value');
        expect(param).toHaveProperty('default_value');
        expect(param).toHaveProperty('is_runtime_configurable');
      }
    });

    test('catalog total_parameters matches actual count', async ({ page }) => {
      const backendUrl = getBackendUrl();
      const response = await page.request.get(`${backendUrl}/api/admin/config/catalog`);

      const data = await response.json();

      // Count actual parameters across all categories
      const actualCount = data.data.categories.reduce(
        (sum: number, cat: { parameters: unknown[] }) => sum + cat.parameters.length,
        0
      );

      expect(data.data.total_parameters).toBe(actualCount);
    });
  });

  test.describe('Configuration UI Integration', () => {
    test('Configuration tab loads with real data', async ({ page }) => {
      await navigateToTab(page, 'Tool Management');

      // Wait for content to load
      await page.waitForLoadState('networkidle', { timeout: timeouts.medium }).catch(() => {});

      // Should show configuration header (Dashboard h1 shows tab name)
      const header = await page.getByRole('heading', { name: 'Tool Management', level: 1 }).isVisible();
      expect(header).toBe(true);
    });

    test('tool list displays from real backend data', async ({ page }) => {
      await navigateToTab(page, 'Tool Management');
      await page.waitForLoadState('networkidle', { timeout: timeouts.medium }).catch(() => {});

      // A card proves nothing here: the plan card is always drawn and the failure
      // notice is a card too. The server's tool catalog fills the table, a row per tool.
      await expect(toolToggles(page)).not.toHaveCount(0, { timeout: timeouts.medium });
      await expect(page.getByText('Failed to load tool availability.')).toHaveCount(0);
    });

    test('search filters the tool list with real data', async ({ page }) => {
      await navigateToTab(page, 'Tool Management');
      await page.waitForLoadState('networkidle', { timeout: timeouts.medium }).catch(() => {});

      const searchInput = page.getByPlaceholder('Search tools by name, description...');
      await expect(searchInput).toBeVisible({ timeout: timeouts.medium });
      await expect(toolToggles(page)).not.toHaveCount(0, { timeout: timeouts.medium });

      // No tool's name or description holds this, so the list empties and says so
      await searchInput.fill('no-tool-is-named-like-this-1589');
      await expect(toolToggles(page)).toHaveCount(0);
      await expect(page.getByText('No tools found matching your criteria.')).toBeVisible();
    });
  });

  test.describe('Configuration Update API', () => {
    test('update endpoint requires authentication', async ({ page }) => {
      const backendUrl = getBackendUrl();

      // No session, and the write header a session would send: the 401 is the missing session
      const headers = await endSessionKeepingCsrfHeader(page);
      const response = await page.request.put(`${backendUrl}/api/admin/config`, {
        headers,
        data: {
          parameters: { 'test.param': 123 },
        },
      });

      // Should fail without auth
      expect(response.status()).toBe(401);
    });

    test('update endpoint validates parameter values', async ({ page }) => {
      const backendUrl = getBackendUrl();

      // A signed-in write, as the SPA sends one, naming a key the catalog does not hold
      const response = await page.request.put(`${backendUrl}/api/admin/config`, {
        headers: await csrfHeader(page),
        data: {
          parameters: { 'nonexistent.invalid.key': 'bad_value' },
        },
      });

      // The request reached validation: an unknown key is a validation error, not a lost session
      expect(response.status()).toBe(400);
      const body = await response.json();
      expect(body.success).toBe(false);
    });
  });

  test.describe('Configuration Reset API', () => {
    test('reset endpoint requires authentication', async ({ page }) => {
      const backendUrl = getBackendUrl();

      // No session, and the write header a session would send: the 401 is the missing session
      const headers = await endSessionKeepingCsrfHeader(page);
      const response = await page.request.post(`${backendUrl}/api/admin/config/reset`, {
        headers,
        data: {
          category: 'tsb',
        },
      });

      expect(response.status()).toBe(401);
    });

    test('reset by category validates category name', async ({ page }) => {
      const backendUrl = getBackendUrl();

      const response = await page.request.post(`${backendUrl}/api/admin/config/reset`, {
        headers: await csrfHeader(page),
        data: {
          category: 'invalid_nonexistent_category',
        },
      });

      // The request reached the handler: a category the catalog does not hold is not found
      expect(response.status()).toBe(404);
    });
  });

  test.describe('Access Control', () => {
    test('catalog endpoint requires authentication', async ({ page }) => {
      const backendUrl = getBackendUrl();

      // Clear all authentication (cookies and localStorage) to truly test unauthenticated access
      await page.context().clearCookies();
      await page.evaluate(() => localStorage.removeItem('pierre_auth_token'));

      const response = await page.request.get(`${backendUrl}/api/admin/config/catalog`);

      expect(response.status()).toBe(401);
    });
  });
});

/**
 * The admin configuration surface belongs to an admin. A plain user who is
 * signed in holds a real session, so the refusal is 403 with the admin
 * guard's sentence, never the 401 a missing session gets: the user is known
 * and is not allowed.
 */
test.describe('Admin Configuration: a signed-in plain user', () => {
  /** What the admin guard answers a signed-in account that is not an admin. */
  const ADMIN_REQUIRED = 'Admin privileges required';

  test.beforeEach(async ({ page }) => {
    const loginResult = await createAndLoginTestUser(page, testUsers.regularUser);
    expect(loginResult.error, 'the plain user signs in').toBeUndefined();
    expect(loginResult.success).toBe(true);
  });

  test('is refused the configuration catalog with 403', async ({ page }) => {
    const backendUrl = getBackendUrl();

    const response = await page.request.get(`${backendUrl}/api/admin/config/catalog`);

    expect(response.status()).toBe(403);
    const body = await response.json();
    expect(body.message).toBe(ADMIN_REQUIRED);
    expect(body.data, 'a refusal carries no catalog').toBeUndefined();
  });

  test('is refused the current configuration with 403', async ({ page }) => {
    const backendUrl = getBackendUrl();

    const response = await page.request.get(`${backendUrl}/api/admin/config`);

    expect(response.status()).toBe(403);
    expect((await response.json()).message).toBe(ADMIN_REQUIRED);
  });

  test('is refused a configuration update with 403', async ({ page }) => {
    const backendUrl = getBackendUrl();

    // The write header a session sends, so the refusal is the handler's and not the CSRF check's 401
    const response = await page.request.put(`${backendUrl}/api/admin/config`, {
      headers: await csrfHeader(page),
      data: {
        parameters: { 'usage_quotas.max_coaches_per_user': 4 },
        reason: 'carnet#1589: a plain user is refused',
      },
    });

    expect(response.status()).toBe(403);
    expect((await response.json()).message).toBe(ADMIN_REQUIRED);
  });

  test('is refused a configuration reset with 403', async ({ page }) => {
    const backendUrl = getBackendUrl();

    const response = await page.request.post(`${backendUrl}/api/admin/config/reset`, {
      headers: await csrfHeader(page),
      data: {
        category: 'usage_quotas',
      },
    });

    expect(response.status()).toBe(403);
    expect((await response.json()).message).toBe(ADMIN_REQUIRED);
  });
});
