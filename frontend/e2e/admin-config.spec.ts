// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Playwright E2E tests for Admin Tool Management tab access control.
// ABOUTME: Validates tab visibility for admin vs non-admin users.

import { test, expect, type Page } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard } from './test-helpers';

// Minimal mock setup for admin dashboard with tool management endpoints
async function setupAdminMocks(page: Page) {
  await setupDashboardMocks(page, { role: 'admin' });

  // Mock tool availability endpoints used by ToolAvailability component
  await page.route('**/api/admin/tools/catalog', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ tools: [] }) }),
  );
  await page.route('**/api/admin/tools/global-disabled', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ tools: [] }) }),
  );
  await page.route('**/api/admin/tools/tenant/**', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ tools: [] }) }),
  );
  // Mock admin settings endpoints rendered alongside tool management
  await page.route('**/api/admin/settings/auto-approval', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ data: { enabled: false, description: 'Auto-approval disabled' } }),
    }),
  );
  await page.route('**/api/admin/settings/social-insights', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        data: {
          min_relevance_score: 70,
          activity_fetch_limits: { insight_context_limit: 50, training_context_limit: 10, max_client_limit: 200 },
          streak_config: { lookback_days: 30, min_for_sharing: 3 },
          milestone_thresholds: { min_activities_for_milestone: 10 },
        },
      }),
    }),
  );
}

test.describe('Admin Tool Management - Access Control', () => {
  test('non-admin users cannot see Tool Management tab', async ({ page }) => {
    await setupDashboardMocks(page, { role: 'user' });
    await loginToDashboard(page);
    await page.waitForSelector('aside', { timeout: 10000 });

    await expect(page.getByRole('button', { name: 'Settings', exact: true }).first()).toBeVisible();
    await expect(page.locator('button').filter({ has: page.locator('span:has-text("Tool Management")') })).not.toBeVisible();
  });

  test('admin users can see Tool Management tab', async ({ page }) => {
    await setupAdminMocks(page);
    await loginToDashboard(page);
    await page.waitForSelector('nav', { timeout: 10000 });

    await expect(page.locator('button').filter({ has: page.locator('span:has-text("Tool Management")') })).toBeVisible();
  });
});
