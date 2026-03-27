// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Playwright E2E tests for the Engagement tab (previously Tool Usage).
// ABOUTME: Tests coach leaderboard, user engagement tiers, time range selector, and empty states.

import { test, expect, type Page } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard, navigateToTab } from './test-helpers';

// Helper to set up authenticated state with Engagement API mocks
async function setupEngagementMocks(
  page: Page,
  options: {
    hasData?: boolean;
  } = {}
) {
  const { hasData = true } = options;

  // Set up base dashboard mocks (includes login mock)
  await setupDashboardMocks(page, { role: 'admin' });

  // Mock system coaches endpoint (used by coach leaderboard)
  await page.route('**/api/admin/coaches', async (route) => {
    if (!hasData) {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ coaches: [], total: 0, metadata: { timestamp: new Date().toISOString(), api_version: '1.0' } }),
      });
      return;
    }

    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        coaches: [
          { id: 'c1', title: 'Marathon Coach', category: 'running', token_count: 15000, description: 'Marathon training' },
          { id: 'c2', title: 'Recovery Advisor', category: 'recovery', token_count: 8500, description: 'Recovery guidance' },
          { id: 'c3', title: 'Nutrition Planner', category: 'nutrition', token_count: 5200, description: 'Meal planning' },
        ],
        total: 3,
        metadata: { timestamp: new Date().toISOString(), api_version: '1.0' },
      }),
    });
  });

  // Mock all users endpoint (used for engagement tiers)
  await page.route('**/api/admin/users*', async (route) => {
    if (!hasData) {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ users: [] }),
      });
      return;
    }

    const now = Date.now();
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        users: [
          { id: '1', email: 'active1@test.com', last_active: new Date(now - 3600000).toISOString() },
          { id: '2', email: 'active2@test.com', last_active: new Date(now - 7200000).toISOString() },
          { id: '3', email: 'weekly@test.com', last_active: new Date(now - 3 * 86400000).toISOString() },
          { id: '4', email: 'monthly@test.com', last_active: new Date(now - 15 * 86400000).toISOString() },
          { id: '5', email: 'dormant@test.com', last_active: new Date(now - 60 * 86400000).toISOString() },
        ],
      }),
    });
  });

  // Mock store stats
  await page.route('**/api/admin/store/stats', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        pending_count: 0,
        published_count: hasData ? 5 : 0,
        rejected_count: 0,
        total_installs: 0,
        rejection_rate: 0,
      }),
    });
  });
}

async function loginAndNavigateToEngagement(page: Page) {
  await loginToDashboard(page);
  await navigateToTab(page, 'Engagement');
  await page.waitForTimeout(500);
}

test.describe('Engagement Tab - Overview', () => {
  test('renders Engagement tab with header', async ({ page }) => {
    await setupEngagementMocks(page);
    await loginAndNavigateToEngagement(page);

    // Check header — Dashboard top bar shows the active tab name
    await expect(page.getByRole('heading', { name: 'Engagement', level: 1 })).toBeVisible();
  });

  test('displays time range selector', async ({ page }) => {
    await setupEngagementMocks(page);
    await loginAndNavigateToEngagement(page);

    await expect(page.getByText('Time Range:')).toBeVisible();

    // Use button role to target only the pill buttons, avoiding "Requests (7 Days)" label
    await expect(page.getByRole('button', { name: '7 Days' })).toBeVisible();
    await expect(page.getByRole('button', { name: '14 Days' })).toBeVisible();
    await expect(page.getByRole('button', { name: '30 Days' })).toBeVisible();
    await expect(page.getByRole('button', { name: '90 Days' })).toBeVisible();
  });
});

test.describe('Engagement Tab - User Engagement Tiers', () => {
  test('displays Daily Active stat card', async ({ page }) => {
    await setupEngagementMocks(page);
    await loginAndNavigateToEngagement(page);

    await expect(page.getByText('Daily Active')).toBeVisible();
    // 2 users active within last 24h
    await expect(page.locator('.stat-card-dark').filter({ hasText: 'Daily Active' }).getByText('2')).toBeVisible();
  });

  test('displays Weekly Active stat card', async ({ page }) => {
    await setupEngagementMocks(page);
    await loginAndNavigateToEngagement(page);

    await expect(page.getByText('Weekly Active')).toBeVisible();
    // 1 user active within last 7 days (but not last 24h)
    await expect(page.locator('.stat-card-dark').filter({ hasText: 'Weekly Active' }).getByText('1')).toBeVisible();
  });

  test('displays Monthly Active stat card', async ({ page }) => {
    await setupEngagementMocks(page);
    await loginAndNavigateToEngagement(page);

    await expect(page.getByText('Monthly Active')).toBeVisible();
  });

  test('displays Dormant stat card', async ({ page }) => {
    await setupEngagementMocks(page);
    await loginAndNavigateToEngagement(page);

    await expect(page.getByText('Dormant')).toBeVisible();
  });
});

test.describe('Engagement Tab - Coach Leaderboard', () => {
  test('displays Coach Leaderboard section', async ({ page }) => {
    await setupEngagementMocks(page);
    await loginAndNavigateToEngagement(page);

    await expect(page.getByText('Coach Leaderboard')).toBeVisible();
  });

  test('displays coach names in leaderboard', async ({ page }) => {
    await setupEngagementMocks(page);
    await loginAndNavigateToEngagement(page);

    await expect(page.getByText('Marathon Coach')).toBeVisible();
    await expect(page.getByText('Recovery Advisor')).toBeVisible();
    await expect(page.getByText('Nutrition Planner')).toBeVisible();
  });

  test('displays category badges', async ({ page }) => {
    await setupEngagementMocks(page);
    await loginAndNavigateToEngagement(page);

    // Use exact matching to target the category badge text, not coach names
    // that contain the category as a substring (e.g., "Recovery Advisor" vs "recovery" badge)
    const leaderboard = page.locator('table');
    await expect(leaderboard.getByText('running', { exact: true })).toBeVisible();
    await expect(leaderboard.getByText('recovery', { exact: true })).toBeVisible();
    await expect(leaderboard.getByText('nutrition', { exact: true })).toBeVisible();
  });

  test('displays token counts in leaderboard', async ({ page }) => {
    await setupEngagementMocks(page);
    await loginAndNavigateToEngagement(page);

    // Token counts are formatted with commas
    await expect(page.getByText('15,000')).toBeVisible();
    await expect(page.getByText('8,500')).toBeVisible();
  });

  test('coaches are ranked by token usage', async ({ page }) => {
    await setupEngagementMocks(page);
    await loginAndNavigateToEngagement(page);

    // Marathon Coach should be #1 (most tokens)
    const rows = page.locator('tbody tr');
    await expect(rows.first()).toContainText('Marathon Coach');
    await expect(rows.nth(1)).toContainText('Recovery Advisor');
    await expect(rows.nth(2)).toContainText('Nutrition Planner');
  });
});

test.describe('Engagement Tab - Platform Stats', () => {
  test('displays Platform Stats section', async ({ page }) => {
    await setupEngagementMocks(page);
    await loginAndNavigateToEngagement(page);

    await expect(page.getByText('Platform Stats')).toBeVisible();
  });

  test('displays Total Users count', async ({ page }) => {
    await setupEngagementMocks(page);
    await loginAndNavigateToEngagement(page);

    await expect(page.getByText('Total Users')).toBeVisible();
  });

  test('displays Total Coaches count', async ({ page }) => {
    await setupEngagementMocks(page);
    await loginAndNavigateToEngagement(page);

    await expect(page.getByText('Total Coaches')).toBeVisible();
  });

  test('displays Published Coaches count', async ({ page }) => {
    await setupEngagementMocks(page);
    await loginAndNavigateToEngagement(page);

    await expect(page.getByText('Published Coaches')).toBeVisible();
  });
});

test.describe('Engagement Tab - Empty State', () => {
  test('shows empty state when no data', async ({ page }) => {
    await setupEngagementMocks(page, { hasData: false });
    await loginAndNavigateToEngagement(page);

    // Matches the EngagementTab empty state text
    await expect(page.getByText('No coach activity yet')).toBeVisible();
    await expect(page.getByText('Create system coaches to get started.')).toBeVisible();
  });
});

test.describe('Engagement Tab - Time Range Interaction', () => {
  test('clicking 30 Days changes selection', async ({ page }) => {
    await setupEngagementMocks(page);
    await loginAndNavigateToEngagement(page);

    const thirtyDayButton = page.locator('button').filter({ hasText: '30 Days' });
    await thirtyDayButton.click();
    await page.waitForTimeout(300);

    // The button should now have the active styling
    await expect(thirtyDayButton).toHaveClass(/pierre-violet/);
  });

  test('clicking 90 Days changes selection', async ({ page }) => {
    await setupEngagementMocks(page);
    await loginAndNavigateToEngagement(page);

    const ninetyDayButton = page.locator('button').filter({ hasText: '90 Days' });
    await ninetyDayButton.click();
    await page.waitForTimeout(300);

    // The button should now have the active styling
    await expect(ninetyDayButton).toHaveClass(/pierre-violet/);
  });
});

test.describe('Engagement Tab - Loading State', () => {
  test('shows loading spinner while data loads', async ({ page }) => {
    await setupDashboardMocks(page, { role: 'admin' });

    await page.route('**/api/admin/coaches', async (route) => {
      await new Promise((resolve) => setTimeout(resolve, 1000));
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ coaches: [], total: 0, metadata: { timestamp: new Date().toISOString(), api_version: '1.0' } }),
      });
    });

    await page.route('**/api/admin/users*', async (route) => {
      await new Promise((resolve) => setTimeout(resolve, 1000));
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ users: [] }),
      });
    });

    await loginToDashboard(page);
    await navigateToTab(page, 'Engagement');

    // Should show loading spinner
    await expect(page.locator('.pierre-spinner')).toBeVisible({ timeout: 5000 });
  });
});
