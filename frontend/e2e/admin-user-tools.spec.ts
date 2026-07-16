// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Playwright E2E tests for the per-user tool override flow in the User Tools admin tab.
// ABOUTME: Covers disable toggle, user_override badge, persistence across reload, and reset to default.

import { test, expect, type Page } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard, navigateToTab } from './test-helpers';

// Seeded non-admin user the flow operates on (mirrors the dev-seed alice@acme.com)
const alice = {
  id: 'user-alice',
  email: 'alice@acme.com',
  display_name: 'Alice Athlete',
  is_admin: false,
  role: 'user',
  user_status: 'active',
  tier: 'starter',
  tenant_id: 'tenant-acme',
  created_at: '2026-01-01T00:00:00Z',
};

// Catalog defaults for alice's effective tools; overrides overlay on top
const baseTools = [
  {
    tool_name: 'get_activities',
    display_name: 'Get Activities',
    description: 'Retrieve fitness activities from connected providers',
    category: 'fitness',
    is_enabled: true,
    source: 'default',
    min_plan: 'starter',
  },
  {
    tool_name: 'analyze_activity',
    display_name: 'Analyze Activity',
    description: 'Perform deep analysis on a single activity',
    category: 'analysis',
    is_enabled: true,
    source: 'default',
    min_plan: 'starter',
  },
  {
    tool_name: 'set_configuration',
    display_name: 'Set Configuration',
    description: 'Modify runtime configuration parameters',
    category: 'configuration',
    is_enabled: false,
    source: 'global_disabled',
    min_plan: 'starter',
  },
];

/**
 * Registers stateful mocks for the per-user tool endpoints. The overrides
 * map lives in the test closure, so POST/DELETE mutations survive a
 * page.reload() — the persistence assertion exercises a real refetch of
 * mutated backend state, not a static fixture.
 */
async function setupUserToolsMocks(page: Page): Promise<Map<string, boolean>> {
  await setupDashboardMocks(page, { role: 'admin' });

  const overrides = new Map<string, boolean>();

  // Session restore after page.reload() — same user the /oauth/token mock returns
  await page.route('**/api/auth/session', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        access_token: 'test-jwt-token',
        csrf_token: 'test-csrf-token',
        user: {
          id: 'user-123',
          user_id: 'user-123',
          email: 'admin@test.com',
          display_name: 'Test Admin',
          role: 'admin',
          is_admin: true,
          user_status: 'active',
          tier: 'professional',
          tenant_id: 'user-123',
        },
      }),
    }),
  );

  // User picker source — registered after setupDashboardMocks so it wins
  await page.route('**/api/admin/users**', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ users: [alice], total_count: 1 }),
    }),
  );

  // Effective tools for a user: base catalog with the override overlay applied.
  // Regex excludes the /override subpaths (they have extra segments).
  await page.route(/\/api\/admin\/tools\/user\/[^/]+$/, async (route) => {
    if (route.request().method() === 'GET') {
      const data = baseTools.map((tool) =>
        overrides.has(tool.tool_name)
          ? { ...tool, is_enabled: overrides.get(tool.tool_name), source: 'user_override' }
          : tool,
      );
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ success: true, message: 'Effective tools retrieved', data }),
      });
    } else {
      await route.fallback();
    }
  });

  // Set per-user override — records into the stateful map
  await page.route('**/api/admin/tools/user/*/override', async (route) => {
    if (route.request().method() === 'POST') {
      const body = route.request().postDataJSON();
      overrides.set(body.tool_name, body.is_enabled);
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          success: true,
          message: 'User tool override set',
          data: {
            user_id: alice.id,
            tool_name: body.tool_name,
            is_enabled: body.is_enabled,
            set_by: 'user-123',
            reason: body.reason ?? null,
            created_at: new Date().toISOString(),
            updated_at: new Date().toISOString(),
          },
        }),
      });
    } else {
      await route.fallback();
    }
  });

  // Remove per-user override — deletes from the stateful map
  await page.route('**/api/admin/tools/user/*/override/*', async (route) => {
    if (route.request().method() === 'DELETE') {
      const url = route.request().url();
      const toolName = url.slice(url.lastIndexOf('/') + 1);
      overrides.delete(toolName);
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ success: true, message: 'User tool override removed' }),
      });
    } else {
      await route.fallback();
    }
  });

  return overrides;
}

/** Opens the User Tools tab and selects alice, waiting for her tool list. */
async function openUserToolsForAlice(page: Page) {
  await page.waitForSelector('nav', { timeout: 10000 });
  await navigateToTab(page, 'User Tools');
  await expect(page.getByRole('button', { name: /alice@acme.com/ })).toBeVisible({
    timeout: 10000,
  });
  await page.getByRole('button', { name: /alice@acme.com/ }).click();
  await expect(page.getByText('Get Activities')).toBeVisible({ timeout: 10000 });
}

test.describe('Admin User Tools - Per-User Override Flow', () => {
  test('disable a tool, persist across reload, then reset to default', async ({ page }) => {
    const overrides = await setupUserToolsMocks(page);
    await loginToDashboard(page);

    const row = page.locator('li').filter({ hasText: 'get_activities' });
    const toggle = page.getByRole('switch', { name: 'Toggle Get Activities' });
    const resetButton = row.locator(
      'button[title="Remove override (revert to plan/tenant/default)"]',
    );

    try {
      await openUserToolsForAlice(page);

      // Baseline: enabled with the catalog-default badge, no reset affordance
      await expect(toggle).toHaveAttribute('aria-checked', 'true');
      await expect(row.getByText('Default', { exact: true })).toBeVisible();
      await expect(resetButton).not.toBeVisible();

      // Toggle OFF → row flips to disabled with the user-override badge
      await toggle.click();
      await expect(toggle).toHaveAttribute('aria-checked', 'false');
      await expect(row.getByText('User Override', { exact: true })).toBeVisible();
      await expect(resetButton).toBeVisible();

      // Persistence: reload, reopen the panel for the same user — the
      // stateful mock serves the stored override back to the fresh fetch
      await page.reload();
      await page.waitForSelector('main', { timeout: 10000 });
      await openUserToolsForAlice(page);

      await expect(toggle).toHaveAttribute('aria-checked', 'false');
      await expect(row.getByText('User Override', { exact: true })).toBeVisible();

      // Reset: remove the override → badge returns to Default, tool re-enables
      await resetButton.click();
      await expect(row.getByText('Default', { exact: true })).toBeVisible();
      await expect(toggle).toHaveAttribute('aria-checked', 'true');
      await expect(resetButton).not.toBeVisible();
      expect(overrides.size).toBe(0);
    } finally {
      // Self-cleaning: if an assertion failed mid-flow, still attempt the UI
      // reset so the flow leaves no override behind, then clear mock state.
      const needsUiReset = await resetButton.isVisible().catch(() => false);
      if (needsUiReset) {
        await resetButton.click().catch(() => {});
      }
      overrides.clear();
    }
  });

  test('globally disabled tools stay locked in the per-user panel', async ({ page }) => {
    await setupUserToolsMocks(page);
    await loginToDashboard(page);
    await openUserToolsForAlice(page);

    const globalRow = page.locator('li').filter({ hasText: 'set_configuration' });
    await expect(globalRow.getByText('Global Block', { exact: true })).toBeVisible();
    await expect(page.getByRole('switch', { name: 'Toggle Set Configuration' })).toBeDisabled();
  });

  test('non-admin users cannot see the User Tools tab', async ({ page }) => {
    await setupDashboardMocks(page, { role: 'user' });
    await loginToDashboard(page);
    await page.waitForSelector('aside', { timeout: 10000 });

    await expect(page.getByRole('button', { name: 'Settings', exact: true }).first()).toBeVisible();
    await expect(
      page.locator('button').filter({ has: page.locator('span:has-text("User Tools")') }),
    ).not.toBeVisible();
  });
});
